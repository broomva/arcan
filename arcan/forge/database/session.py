#%%
import contextlib
import os
from contextlib import asynccontextmanager
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

print(settings)

class DatabaseSessionManager:
    def __init__(self, host: str = None, engine: AsyncEngine = None) -> None:
        self.engine: AsyncEngine | engine | engine = create_async_engine(host, connect_args={"statement_cache_size": 0})  # Disable statement caching
        self._sessionmaker: async_sessionmaker[AsyncSession] = async_sessionmaker(
            bind=self.engine, class_=AsyncSession, expire_on_commit=False,
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




engine = create_async_engine(
    settings.database_url,
    echo=True,
    connect_args={"statement_cache_size": 0}  # Disable statement caching
)

sessionmanager = DatabaseSessionManager(host=settings.database_url, engine=engine)
Base = declarative_base()

@asynccontextmanager
async def session_scope():
    async with sessionmanager.session() as session:
        yield session
        
        
# @asynccontextmanager
# async def session_scope() -> AsyncSession:
#     async with sessionmanager() as session:
#         try:
#             yield session
#             await session.commit()
#         except Exception:
#             await session.rollback()
#             raise
#         finally:
#             await session.close()





# # class Config:
# #     DATABASE_URL = os.getenv("DATABASE_URL").replace("postgresql://", "postgresql+asyncpg://")
# #     ENVIRONMENT = os.getenv("ENVIRONMENT")

# # database/session.py
# class EngineFactory:
#     def __init__(self):
#         self.engines = {"local": self.local_engine, "cloud": self.cloud_engine}

#     def get_engine(self):
#         # Fetch the appropriate engine creation method from the dictionary
#         engine_type = (
#             settings.environment or "cloud"
#         )  # Default to 'cloud' if not specified
#         engine_creator = self.engines.get(
#             engine_type, self.cloud_engine
#         )  # Fallback to cloud engine
#         return engine_creator()

#     def local_engine(self):
#         """Create a local SQLite engine"""
#         return create_async_engine("sqlite+aiosqlite:///arcan.db")

#     def cloud_engine(self):
#         """Create a cloud engine from a URL in the config"""
#         if not settings.database_url:
#             raise ValueError("No database URL provided for cloud environment.")
#         return create_async_engine(
#             settings.database_url, 
#             echo=True,
#             connect_args={"statement_cache_size": 0}  # Disable statement caching
#         )


# class DatabaseSessionManager:
#     def __init__(self, engine):
#         self.engine: AsyncEngine | engine = EngineFactory().get_engine() 
#         self._sessionmaker: async_sessionmaker[AsyncSession] = async_sessionmaker(
#             autocommit=False, bind=self.engine
#         )

#     async def close(self):
#         if self.engine is None:
#             raise ServiceError
#         await self.engine.dispose()
#         self.engine = None
#         self._sessionmaker = None  # type: ignore

#     @contextlib.asynccontextmanager
#     async def connect(self) -> AsyncIterator[AsyncConnection]:
#         if self.engine is None:
#             raise ServiceError

#         async with self.engine.begin() as connection:
#             try:
#                 yield connection
#             except SQLAlchemyError:
#                 await connection.rollback()
#                 logger.error("Connection error occurred")
#                 raise ServiceError

#     @contextlib.asynccontextmanager
#     async def session(self) -> AsyncIterator[AsyncSession]:
#         if not self._sessionmaker:
#             logger.error("Sessionmaker is not available")
#             raise ServiceError

#         session = self._sessionmaker()
#         try:
#             yield session
#         except SQLAlchemyError as e:
#             await session.rollback()
#             logger.error(f"Session error could not be established {e}")
#             raise ServiceError
#         finally:
#             await session.close()



# #%%
# import os

# from sqlalchemy.ext.asyncio import AsyncSession, create_async_engine
# from sqlalchemy.orm import declarative_base, sessionmaker

# engine = create_async_engine(
#     settings.database_url,
#     echo=True,
# )

# sessionmanager = sessionmaker(
#     bind=engine,
#     class_=AsyncSession,
#     expire_on_commit=False,
# )

# Base = declarative_base()

# # engine = EngineFactory().get_engine()
# # sessionmanager = DatabaseSessionManager(engine=engine)

# # Base = declarative_base()

# @asynccontextmanager
# async def session_scope() -> AsyncSession:
#     async with sessionmanager() as session:
#         try:
#             yield session
#             await session.commit()
#         except Exception:
#             await session.rollback()
#             raise
#         finally:
#             await session.close()

# %%
