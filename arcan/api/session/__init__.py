import ast
import os
import pickle
import weakref
from datetime import datetime
from typing import Dict

from fastapi import Depends
from langchain.sql_database import SQLDatabase
from sqlalchemy.dialects.postgresql import insert
from sqlalchemy.exc import SQLAlchemyError
from sqlalchemy.orm import Session

from arcan.ai.agents import ArcanAgent
from arcan.api.datamodel.chat_history import ChatHistory
from arcan.api.datamodel.conversation import Conversation


class ArcanSession:
    def __init__(self, database: Session):
        """
        Initializes a new instance of the ArcanSession class.

        :param database: A callable that returns a new SQLAlchemy Session instance when called.
        """
        self.database = database
        self.database_uri = os.environ.get("SQLALCHEMY_URL")
        self.agents: Dict[str, weakref.ref] = weakref.WeakValueDictionary()

    def get_or_create_agent(
        self, user_id: str, provided_agent: ArcanAgent = None
    ) -> ArcanAgent:
        """
        Retrieves or creates a ArcanAgent for a given user_id.

        :param user_id: The unique identifier for the user.
        :return: An instance of ArcanAgent.
        """
        if provided_agent is None:
            agent = self.agents.get(user_id)
            chat_history = []

            # Obtain a new database session
            try:
                chat_history = self.get_chat_history(user_id)
            except Exception as e:
                print(f"Error getting chat history for {user_id}: {e}")

            if agent is not None and chat_history:
                print(f"Using existing agent {agent}")
            elif agent is None and chat_history:
                print(f"Using reloaded agent with history {chat_history}")
                agent = ArcanAgent(
                    context=chat_history,
                    user_id=user_id,
                    # database=SQLDatabase.from_uri(self.database_uri) # database tool model
                )  # Initialize with chat history
            elif agent is None and not chat_history:
                print("Using a new agent")
                agent = ArcanAgent(
                    user_id=user_id,
                )
                #   database=SQLDatabase.from_uri(self.database_uri))  # Initialize without chat history

            self.agents[user_id] = agent
            return agent

        else:
            provided_agent.user_id = user_id
            self.agents[user_id] = provided_agent
            return provided_agent

    def store_message(self, user_id: str, body: str, response: str):
        """
        Stores a message in the database.

        :param user_id: The unique identifier for the user.
        :param Body: The body of the message sent by the user.
        :param response: The response generated by the system.
        """
        with self.database as db_session:
            conversation = Conversation(sender=user_id, message=body, response=response)
            db_session.add(conversation)
            db_session.commit()
            print(f"Conversation #{conversation.id} stored in database")

    def store_chat_history(self, user_id, agent_history):
        """
        Stores or updates the chat history for a user in the database.

        :param user_id: The unique identifier for the user.
        :param agent_history: The chat history to be stored.
        """
        history = pickle.dumps(agent_history)
        # Upsert statement
        stmt = (
            insert(ChatHistory)
            .values(
                sender=user_id,
                history=str(history),
                updated_at=datetime.utcnow(),  # Explicitly set updated_at on insert
            )
            .on_conflict_do_update(
                index_elements=["sender"],  # Specify the conflict target
                set_={
                    "history": str(history),  # Update the history field upon conflict
                    "updated_at": datetime.utcnow(),  # Update the updated_at field upon conflict
                },
            )
        )
        # Execute the upsert
        with self.database as db:
            db.execute(stmt)
            db.commit()
            print(f"Upsert chat history for user {user_id} with statement {stmt}")

    def get_chat_history(self, user_id: str) -> list:
        """
        Retrieves the chat history for a user from the database.

        :param db_session: The SQLAlchemy Session instance.
        :param user_id: The unique identifier for the user.
        :return: A list representing the chat history.
        """
        with self.database as db_session:
            history = (
                db_session.query(ChatHistory)
                .filter(ChatHistory.sender == user_id)
                .order_by(ChatHistory.updated_at.asc())
                .all()
            ) or []
        if not history:
            return []
        chat_history = history[0].history
        loaded = pickle.loads(ast.literal_eval(chat_history))
        return loaded


def run_agent(session: ArcanSession, query: str, user_id: str) -> Dict[str, str]:
    print(f"Sending the LangChain response to user: {user_id}")
    agent = session.get_or_create_agent(user_id)
    # Get the generated text from the LangChain agent
    response = agent.get_response(user_content=query)
    # Store the conversation in the database
    try:
        session.store_message(user_id=user_id, body=query, response=response)
        session.store_chat_history(user_id=user_id, agent_history=agent.chat_history)
    except SQLAlchemyError as e:
        session.database.rollback()
        print(f"Error storing conversation in database: {e}")
    return response
