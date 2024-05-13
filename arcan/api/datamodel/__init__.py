# %%
import os
from contextlib import contextmanager

from dotenv import load_dotenv
from sqlalchemy import create_engine
from sqlalchemy.ext.declarative import declarative_base
from sqlalchemy.orm import sessionmaker

load_dotenv()

# %%


DATABASE_URL = str(os.environ.get("SQLALCHEMY_URL"))
print(DATABASE_URL)
assert DATABASE_URL is not None, "SQLALCHEMY_URL environment variable not found"

engine = create_engine(
    DATABASE_URL
)  # Oddly requires the hard coded string or else fails to connect
SessionLocal = sessionmaker(bind=engine)
Base = declarative_base()


def get_db():
    """
    Returns a database session.

    Yields:
        SessionLocal: The database session.

    """
    try:
        db = SessionLocal()
        yield db
    finally:
        db.close()


@contextmanager
def get_db_context():
    """
    Context manager wrapper for the get_db generator.
    """
    try:
        db = next(get_db())  # Get the session from the generator
        yield db
    finally:
        db.close()


# %%
