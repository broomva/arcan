#%%
from dotenv import load_dotenv
from fastapi import Depends, FastAPI, Form, Request
from sqlalchemy.exc import SQLAlchemyError
from sqlalchemy.orm import Session

from arcan.api.datamodels import get_db, get_db_context
from arcan.api.session import ArcanSession, run_agent

#%%
# from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer

# from arcan.api.session.auth import requires_auth

# auth_scheme = HTTPBearer()

load_dotenv()

app = FastAPI()


@app.get("/")
def default():
    return {
        "message": "Check out the API documentation at http://arcanai.tech/api/docs"
    }


@app.get("/api/check")
async def index():
    return {"message": "Arcan is Running!"}





@app.get("/api/chat/{user_id}")
async def api_user_chat(user_id: str, query: str, db: Session = Depends(get_db)):
    arcan_session = ArcanSession(db)
    response = run_agent(session=arcan_session, user_id=user_id, query=query)
    return {"response": response}

# @requires_auth
@app.get("/api/chat")
async def chat(user_id: str, query: str, db: Session = Depends(get_db)):
    arcan_session = ArcanSession(db)
    response = run_agent(session=arcan_session, user_id=user_id, query=query)
    return {"response": response}

#%%