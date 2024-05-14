# %%
from dotenv import load_dotenv
from sqlalchemy.ext.declarative import declarative_base

load_dotenv()

Base = declarative_base()