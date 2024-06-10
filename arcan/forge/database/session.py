#%%
import contextlib
import os
from contextlib import asynccontextmanager, contextmanager
from typing import AsyncIterator

from dotenv import load_dotenv
from loguru import logger
from sqlalchemy.exc import SQLAlchemyError
from sqlalchemy.ext.asyncio import (AsyncConnection, AsyncEngine, AsyncSession,
                                    async_sessionmaker, create_async_engine)
from sqlalchemy.ext.declarative import declarative_base

from arcan.forge.config import settings
from arcan.forge.exceptions import ServiceError

load_dotenv()

class Config:
    DATABASE_URL = os.getenv("DATABASE_URL").replace("postgresql://", "postgresql+asyncpg://")
    ENVIRONMENT = os.getenv("ENVIRONMENT")

class EngineFactory:
    def __init__(self):
        self.engines = {"local": self.local_engine, "cloud": self.cloud_engine}

    def get_engine(self):
        # Fetch the appropriate engine creation method from the dictionary
        engine_type = (
            Config.ENVIRONMENT or "cloud"
        )  # Default to 'cloud' if not specified
        engine_creator = self.engines.get(
            engine_type, self.cloud_engine
        )  # Fallback to cloud engine
        return engine_creator()

    def local_engine(self):
        """Create a local SQLite engine"""
        return create_async_engine("sqlite+aiosqlite:///arcan.db")

    def cloud_engine(self):
        """Create a cloud engine from a URL in the config"""
        if not Config.DATABASE_URL:
            raise ValueError("No database URL provided for cloud environment.")
        return create_async_engine(Config.DATABASE_URL, echo=True)

class DatabaseSessionManager:
    def __init__(self, host: str):
        self.engine: AsyncEngine | None = EngineFactory().get_engine() 
        self._sessionmaker: async_sessionmaker[AsyncSession] = async_sessionmaker(
            autocommit=False, bind=self.engine
        )

    async def close(self):
        if self.engine is None:
            raise ServiceError
        await self.engine.dispose()
        self.engine = None
        self._sessionmaker = None  # type: ignore

    @contextlib.asynccontextmanager
    async def connect(self) -> AsyncIterator[AsyncConnection]:
        if self.engine is None:
            raise ServiceError

        async with self.engine.begin() as connection:
            try:
                yield connection
            except SQLAlchemyError:
                await connection.rollback()
                logger.error("Connection error occurred")
                raise ServiceError

    @contextlib.asynccontextmanager
    async def session(self) -> AsyncIterator[AsyncSession]:
        if not self._sessionmaker:
            logger.error("Sessionmaker is not available")
            raise ServiceError

        session = self._sessionmaker()
        try:
            yield session
        except SQLAlchemyError as e:
            await session.rollback()
            logger.error(f"Session error could not be established {e}")
            raise ServiceError
        finally:
            await session.close()
            
            
sessionmanager = DatabaseSessionManager(settings.database_url)

engine = EngineFactory().get_engine()
Base = declarative_base()

@contextlib.asynccontextmanager
async def session_scope() -> AsyncSession:
    async with sessionmanager.session() as session:
        try:
            yield session
            await session.commit()
        except Exception:
            await session.rollback()
            raise
        finally:
            await session.close()



# %%
