import os
from contextlib import contextmanager

from dotenv import load_dotenv
from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker

load_dotenv()

class Config:
    DATABASE_URL = os.getenv("SQLALCHEMY_URL")
    ENVIRONMENT = os.getenv("ENVIRONMENT")


class EngineFactory:
    def __init__(self):
        self.engines = {
            'local': self.local_engine,
            'cloud': self.cloud_engine
        }
    
    def get_engine(self):
        # Fetch the appropriate engine creation method from the dictionary
        engine_type = Config.ENVIRONMENT or 'cloud'  # Default to 'cloud' if not specified
        engine_creator = self.engines.get(engine_type, self.cloud_engine)  # Fallback to cloud engine
        return engine_creator()
    
    def local_engine(self):
        """ Create a local SQLite engine """
        return create_engine('sqlite:////arcan.db')
    
    def cloud_engine(self):
        """ Create a cloud engine from a URL in the config """
        if not Config.DATABASE_URL:
            raise ValueError("No database URL provided for cloud environment.")
        return create_engine(Config.DATABASE_URL)

factory = EngineFactory()

SessionLocal = sessionmaker(bind=factory.get_engine())

@contextmanager
def session_scope():
    """Provide a transactional scope around a series of operations."""
    session = SessionLocal()
    try:
        yield session
        session.commit()
    except:
        session.rollback()
        raise
    finally:
        session.close()