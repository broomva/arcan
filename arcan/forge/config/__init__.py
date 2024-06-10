#%%
import os

from dotenv import load_dotenv
from pydantic_settings import BaseSettings

load_dotenv()


class Settings(BaseSettings):
    database_url: str = os.environ.get("DATABASE_URL")
    test: bool = False
    project_name: str = "Arcan AI"
    secret_key: str = os.environ.get("SECRET_KEY")
    algorithm: str = os.environ.get("ALGORITHM")
    access_token_expire_minutes: int = 60


settings = Settings()  # type: ignore

# %%
