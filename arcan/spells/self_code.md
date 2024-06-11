
## Directory Structure

```
../../
├── CHANGELOG.md
├── CODE_OF_CONDUCT.md
├── Dockerfile
├── LICENSE
├── Makefile
├── README.md
├── alembic
│   ├── README
│   ├── __pycache__
│   │   └── env.cpython-311.pyc
│   ├── env.py
│   ├── script.py.mako
│   └── versions
│       ├── 3a20784556ea_initial_migration.py
│       ├── 5116ec9ce6b0_initial_migration.py
│       ├── 85735f0876ac_initial_migration.py
│       ├── 92cf759b334d_initial_migration.py
│       ├── __pycache__
│       │   ├── 3a20784556ea_initial_migration.cpython-311.pyc
│       │   ├── 5116ec9ce6b0_initial_migration.cpython-311.pyc
│       │   ├── 85735f0876ac_initial_migration.cpython-311.pyc
│       │   ├── 92cf759b334d_initial_migration.cpython-311.pyc
│       │   └── ee65a56927de_initial_migration.cpython-311.pyc
│       └── ee65a56927de_initial_migration.py
├── alembic.ini
├── arcan
│   ├── __init__.py
│   ├── __pycache__
│   │   └── __init__.cpython-311.pyc
│   ├── casters
│   │   ├── ai
│   │   │   ├── agents
│   │   │   │   ├── __init__.py
│   │   │   │   ├── __pycache__
│   │   │   │   │   ├── __init__.cpython-311.pyc
│   │   │   │   │   ├── helpers.cpython-311.pyc
│   │   │   │   │   └── session.cpython-311.pyc
│   │   │   │   ├── helpers.py
│   │   │   │   ├── researcher.py
│   │   │   │   └── session.py
│   │   │   ├── chains
│   │   │   │   └── __init__.py
│   │   │   ├── graphs
│   │   │   │   └── __init__.py
│   │   │   ├── interface
│   │   │   │   ├── app.py
│   │   │   │   └── chainlit.md
│   │   │   ├── llm
│   │   │   │   ├── __init__.py
│   │   │   │   └── __pycache__
│   │   │   │       └── __init__.cpython-311.pyc
│   │   │   ├── parser
│   │   │   │   ├── __init__.py
│   │   │   │   └── __pycache__
│   │   │   │       └── __init__.cpython-311.pyc
│   │   │   ├── prompts
│   │   │   │   ├── __init__.py
│   │   │   │   └── __pycache__
│   │   │   │       └── __init__.cpython-311.pyc
│   │   │   ├── router
│   │   │   │   ├── __init__.py
│   │   │   │   └── routes.py
│   │   │   ├── runnables
│   │   │   │   └── __init__.py
│   │   │   ├── templates
│   │   │   │   └── __init__.py
│   │   │   └── tools
│   │   │       ├── __init__.py
│   │   │       └── __pycache__
│   │   │           └── __init__.cpython-311.pyc
│   │   ├── decorators.py
│   │   ├── graphs
│   │   │   └── __init__.py
│   │   └── observer.py
│   ├── forge
│   │   ├── __init__.py
│   │   ├── __pycache__
│   │   │   └── __init__.cpython-311.pyc
│   │   ├── api
│   │   │   ├── __init__.py
│   │   │   ├── __pycache__
│   │   │   │   └── __init__.cpython-311.pyc
│   │   │   └── routes
│   │   │       ├── __init__.py
│   │   │       ├── __pycache__
│   │   │       │   ├── __init__.cpython-311.pyc
│   │   │       │   ├── auth.cpython-311.pyc
│   │   │       │   ├── casters.cpython-311.pyc
│   │   │       │   ├── chat_history.cpython-311.pyc
│   │   │       │   ├── conversation.cpython-311.pyc
│   │   │       │   ├── router.cpython-311.pyc
│   │   │       │   └── user.cpython-311.pyc
│   │   │       ├── auth.py
│   │   │       ├── casters.py
│   │   │       ├── chat_history.py
│   │   │       ├── conversation.py
│   │   │       ├── router.py
│   │   │       ├── spells.py
│   │   │       └── user.py
│   │   ├── config
│   │   │   ├── __init__.py
│   │   │   └── __pycache__
│   │   │       └── __init__.cpython-311.pyc
│   │   ├── core
│   │   │   ├── __init__.py
│   │   │   ├── __pycache__
│   │   │   │   ├── __init__.cpython-311.pyc
│   │   │   │   ├── config.cpython-311.pyc
│   │   │   │   └── logging.cpython-311.pyc
│   │   │   ├── config.py
│   │   │   └── logging.py
│   │   ├── database
│   │   │   ├── __init__.py
│   │   │   ├── __pycache__
│   │   │   │   ├── __init__.cpython-311.pyc
│   │   │   │   ├── session.cpython-311.pyc
│   │   │   │   └── tables.cpython-311.pyc
│   │   │   ├── session.py
│   │   │   └── tables.py
│   │   ├── entities
│   │   │   └── __init__.py
│   │   ├── exceptions
│   │   │   ├── __init__.py
│   │   │   └── __pycache__
│   │   │       └── __init__.cpython-311.pyc
│   │   ├── models
│   │   │   ├── __init__.py
│   │   │   ├── __pycache__
│   │   │   │   ├── __init__.cpython-311.pyc
│   │   │   │   ├── chat_history.cpython-311.pyc
│   │   │   │   ├── conversation.cpython-311.pyc
│   │   │   │   ├── token.cpython-311.pyc
│   │   │   │   └── user.cpython-311.pyc
│   │   │   ├── chat_history.py
│   │   │   ├── conversation.py
│   │   │   ├── token.py
│   │   │   └── user.py
│   │   ├── repository
│   │   │   ├── __init__.py
│   │   │   ├── __pycache__
│   │   │   │   ├── __init__.cpython-311.pyc
│   │   │   │   ├── chat_history.cpython-311.pyc
│   │   │   │   ├── conversation.cpython-311.pyc
│   │   │   │   ├── token.cpython-311.pyc
│   │   │   │   └── user.cpython-311.pyc
│   │   │   ├── chat_history.py
│   │   │   ├── conversation.py
│   │   │   ├── token.py
│   │   │   └── user.py
│   │   ├── schemas
│   │   │   ├── __init__.py
│   │   │   ├── __pycache__
│   │   │   │   ├── __init__.cpython-311.pyc
│   │   │   │   ├── chat_history.cpython-311.pyc
│   │   │   │   ├── conversation.cpython-311.pyc
│   │   │   │   ├── token.cpython-311.pyc
│   │   │   │   └── user.cpython-311.pyc
│   │   │   ├── chat_history.py
│   │   │   ├── conversation.py
│   │   │   ├── token.py
│   │   │   └── user.py
│   │   └── service
│   │       ├── __init__.py
│   │       ├── __pycache__
│   │       │   ├── __init__.cpython-311.pyc
│   │       │   └── user.cpython-311.pyc
│   │       └── user.py
│   └── spells
│       ├── __init__.py
│       ├── __pycache__
│       │   ├── __init__.cpython-311.pyc
│       │   ├── scrapping.cpython-311.pyc
│       │   └── search.cpython-311.pyc
│       ├── scrapping.py
│       ├── search.py
│       ├── self.py
│       ├── self_code.md
│       └── vector_search.py
├── arcan.db
├── chainlit.md
├── docker-compose.yml
├── notebooks
│   └── runnables.ipynb
├── poetry.lock
├── public
│   ├── arcan_logo.png
│   └── arcan_logo.svg
├── pyproject.toml
└── tests
    ├── __init__.py
    ├── __pycache__
    │   └── __init__.cpython-311.pyc
    └── arcan
        ├── __init__.py
        ├── __pycache__
        │   └── __init__.cpython-311.pyc
        ├── casters
        │   ├── __init__.py
        │   ├── __pycache__
        │   │   └── __init__.cpython-311.pyc
        │   └── ai
        │       ├── __init__.py
        │       ├── __pycache__
        │       │   └── __init__.cpython-311.pyc
        │       ├── llm
        │       │   ├── __init__.py
        │       │   ├── __pycache__
        │       │   │   ├── __init__.cpython-311.pyc
        │       │   │   └── test_llm.cpython-311-pytest-8.2.2.pyc
        │       │   └── test_llm.py
        │       └── runnables
        │           ├── __pycache__
        │           │   └── test_runnables.cpython-311-pytest-8.2.2.pyc
        │           └── test_runnables.py
        └── nexus
            ├── __init__.py
            ├── __pycache__
            │   └── __init__.cpython-311.pyc
            └── api
                ├── __init__.py
                ├── __pycache__
                │   ├── __init__.cpython-311.pyc
                │   └── test_api.cpython-311-pytest-8.2.2.pyc
                └── test_api.py

69 directories, 156 files

```

## ../../tests/__init__.py

```python

```


## ../../tests/arcan/__init__.py

```python

```


## ../../tests/arcan/casters/__init__.py

```python

```


## ../../tests/arcan/casters/ai/__init__.py

```python

```


## ../../tests/arcan/casters/ai/llm/test_llm.py

```python
import os

import pytest
from dotenv import load_dotenv

from arcan.casters.ai.llm import LLM, ChatGroq, ChatOpenAI, LLMFactory, OpenAI

load_dotenv()


def test_create_llm_chatopenai():
    llm = LLMFactory.create_llm("ChatOpenAI", temperature=0.7)
    assert isinstance(llm, ChatOpenAI)
    assert llm.temperature == 0.7
    assert llm.model_name == os.getenv("OPENAI_MODEL", "gpt-3.5-turbo-0125")


def test_create_llm_chattogetherai():
    llm = LLMFactory.create_llm("ChatTogetherAI", temperature=0.7)
    assert isinstance(llm, ChatOpenAI)
    assert llm.temperature == 0.7
    assert llm.model_name == "mistralai/Mixtral-8x7B-Instruct-v0.1"
    assert llm.openai_api_base == "https://api.together.xyz/v1"


def test_create_llm_chatgroq():
    llm = LLMFactory.create_llm("ChatGroq", temperature=0.7)
    assert isinstance(llm, ChatGroq)
    assert llm.temperature == 0.7
    assert llm.model_name == "llama3-8b-8192"


def test_create_llm_not_implemented():
    with pytest.raises(NotImplementedError):
        LLMFactory.create_llm("InvalidProvider")


def test_llm_factory_create_llm_with_known_provider():
    llm = LLMFactory.create_llm(provider="ChatOpenAI")
    assert isinstance(llm, ChatOpenAI)


def test_llm_factory_create_llm_with_unknown_provider():
    with pytest.raises(NotImplementedError):
        LLMFactory.create_llm(provider="UnknownProvider")

```


## ../../tests/arcan/casters/ai/llm/__init__.py

```python

```


## ../../tests/arcan/casters/ai/runnables/test_runnables.py

```python
import os
from unittest.mock import MagicMock

import pytest
from httpx import AsyncClient

from arcan.api import app
from arcan.casters.ai.runnables import ArcanRunnables


@pytest.fixture
def base_url():
    return "http://localhost:8000/"


def test_get_spells_runnable(base_url):
    runnable_factory = MagicMock()
    arcan_runnables = ArcanRunnables(base_url=base_url)
    arcan_runnables.factory = runnable_factory

    arcan_runnables.get_spells_runnable()

    runnable_factory.get_runnable.assert_called_once_with(runnable_name="spells")

    assert arcan_runnables.get_spells_runnable().invoke(
        {"input": "testinggggg$#@"}
    ).json() == {"response": "test"}


def test_get_openai_runnable(base_url):
    runnable_factory = MagicMock()
    arcan_runnables = ArcanRunnables(base_url=base_url)
    arcan_runnables.factory = runnable_factory

    arcan_runnables.get_openai_runnable()

    runnable_factory.get_runnable.assert_called_once_with(runnable_name="openai")


def test_get_groq_runnable(base_url):
    runnable_factory = MagicMock()
    arcan_runnables = ArcanRunnables(base_url=base_url)
    arcan_runnables.factory = runnable_factory

    arcan_runnables.get_groq_runnable()

    runnable_factory.get_runnable.assert_called_once_with(runnable_name="groq")


# def test_get_ollama_runnable(base_url):
#     runnable_factory = MagicMock()
#     arcan_runnables = ArcanRunnables(base_url=base_url)
#     arcan_runnables.factory = runnable_factory

#     arcan_runnables.get_ollama_runnable()

#     runnable_factory.get_runnable.assert_called_once_with(runnable_name="ollama")

```


## ../../tests/arcan/nexus/__init__.py

```python

```


## ../../tests/arcan/nexus/api/__init__.py

```python

```


## ../../tests/arcan/nexus/api/test_api.py

```python
import os

import pytest
from fastapi.testclient import TestClient
from httpx import AsyncClient
from sqlalchemy.orm import Session

from arcan.api import app  # Adjust this import based on your project structure
from arcan.forge.database.session import session_scope


@pytest.mark.asyncio
async def test_redirect_root_to_docs():
    async with AsyncClient(app=app, base_url="http://test") as ac:
        response = await ac.get("/")
        assert response.status_code == 307  # Redirect status code
        assert response.headers["location"] == "/docs"


@pytest.mark.asyncio
async def test_index():
    async with AsyncClient(app=app, base_url="http://test") as ac:
        response = await ac.get("/api/check")
        assert response.status_code == 200
        assert response.json() == {"message": "Arcan is Running!"}


from unittest.mock import MagicMock, patch


@pytest.mark.asyncio
@patch("arcan.forge.database.session")  # Correct the import path as necessary
async def test_chat(mock_session_scope):
    # Create a mock session
    mock_session = MagicMock()
    mock_session_scope.return_value = mock_session

    # Mock specific behaviors, e.g., query handling
    # mock_session.query.return_value.filter.return_value.one.return_value = YourUserModel(id="1", name="Test User")

    # # Set up a test response for `run_agent` if needed
    # with patch('your_module_path.run_agent') as mock_run_agent:
    #     mock_run_agent.return_value = "Test Response"

    mock_token = MagicMock()
    mock_token.credentials = os.getenv("ARCANAI_API_KEY")

    async with AsyncClient(app=app, base_url="http://test") as ac:
        response = await ac.get(
            "/api/chat",
            params={"user_id": "test_user", "query": "testinggggg$#@"},
            headers={"Authorization ": f"Bearer {mock_token.credentials}"},
        )
        assert response.status_code == 200
        assert response.json() == {"response": "test"}


# def test_llm_endpoints():
#     response = client.get("/openai")
#     assert response.status_code == 200

#     response = client.get("/groq")
#     assert response.status_code == 200

#     response = client.get("/together")
#     assert response.status_code == 200


# # Initialize the test client
# client = TestClient(app)

# def test_redirect_root_to_docs():
#     response = client.get("/")
#     assert response.status_code == 307
#     assert response.headers["location"] == "/docs"

# def test_check_api():
#     response = client.get("/api/check")
#     assert response.status_code == 200
#     assert response.json() == {"message": "Arcan is Running!"}

# def test_chat_api():
#     user_id = "test_user"
#     query = "Hello, Arcan!"
#     response = client.get(f"/api/chat?user_id={user_id}&query={query}")
#     assert response.status_code == 200
#     assert "response" in response.json()

```


## ../../alembic/env.py

```python
import asyncio
from logging.config import fileConfig

from sqlalchemy import MetaData, pool
from sqlalchemy.engine import Connection
from sqlalchemy.ext.asyncio import async_engine_from_config

from alembic import context
from arcan.forge.config import settings

# this is the Alembic Config object, which provides
# access to the values within the .ini file in use.
config = context.config

# Interpret the config file for Python logging.
# This line sets up loggers basically.
if config.config_file_name is not None:
    fileConfig(config.config_file_name)

# add your model's MetaData object here
# for 'autogenerate' support
# from myapp import mymodel
# target_metadata = mymodel.Base.metadata
target_metadata = MetaData(settings.database_url)


# other values from the config, defined by the needs of env.py,
# can be acquired:
# my_important_option = config.get_main_option("my_important_option")
# ... etc.


def run_migrations_offline() -> None:
    """Run migrations in 'offline' mode.

    This configures the context with just a URL
    and not an Engine, though an Engine is acceptable
    here as well.  By skipping the Engine creation
    we don't even need a DBAPI to be available.

    Calls to context.execute() here emit the given string to the
    script output.

    """
    url = config.get_main_option("sqlalchemy.url")
    context.configure(
        url=url,
        target_metadata=target_metadata,
        literal_binds=True,
        dialect_opts={"paramstyle": "named"},
    )

    with context.begin_transaction():
        context.run_migrations()


def do_run_migrations(connection: Connection) -> None:
    context.configure(connection=connection, target_metadata=target_metadata)

    with context.begin_transaction():
        context.run_migrations()


async def run_async_migrations() -> None:
    """In this scenario we need to create an Engine
    and associate a connection with the context.

    """

    connectable = async_engine_from_config(
        config.get_section(config.config_ini_section, {}),
        prefix="sqlalchemy.",
        poolclass=pool.NullPool,
    )

    async with connectable.connect() as connection:
        await connection.run_sync(do_run_migrations)

    await connectable.dispose()


def run_migrations_online() -> None:
    """Run migrations in 'online' mode."""

    asyncio.run(run_async_migrations())


if context.is_offline_mode():
    run_migrations_offline()
else:
    run_migrations_online()

```


## ../../alembic/versions/85735f0876ac_initial_migration.py

```python
"""Initial migration

Revision ID: 85735f0876ac
Revises: 
Create Date: 2024-06-10 14:36:50.217921

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision: str = '85735f0876ac'
down_revision: Union[str, None] = None
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    pass
    # ### end Alembic commands ###


def downgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    pass
    # ### end Alembic commands ###

```


## ../../alembic/versions/ee65a56927de_initial_migration.py

```python
"""Initial migration

Revision ID: ee65a56927de
Revises: 5116ec9ce6b0
Create Date: 2024-06-10 16:31:51.390972

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision: str = 'ee65a56927de'
down_revision: Union[str, None] = '5116ec9ce6b0'
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    pass
    # ### end Alembic commands ###


def downgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    pass
    # ### end Alembic commands ###

```


## ../../alembic/versions/3a20784556ea_initial_migration.py

```python
"""Initial migration

Revision ID: 3a20784556ea
Revises: 85735f0876ac
Create Date: 2024-06-10 14:59:55.248146

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision: str = '3a20784556ea'
down_revision: Union[str, None] = '85735f0876ac'
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    pass
    # ### end Alembic commands ###


def downgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    pass
    # ### end Alembic commands ###

```


## ../../alembic/versions/92cf759b334d_initial_migration.py

```python
"""Initial migration

Revision ID: 92cf759b334d
Revises: ee65a56927de
Create Date: 2024-06-10 16:34:32.482304

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision: str = '92cf759b334d'
down_revision: Union[str, None] = 'ee65a56927de'
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    pass
    # ### end Alembic commands ###


def downgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    pass
    # ### end Alembic commands ###

```


## ../../alembic/versions/5116ec9ce6b0_initial_migration.py

```python
"""Initial migration

Revision ID: 5116ec9ce6b0
Revises: 3a20784556ea
Create Date: 2024-06-10 15:02:50.886729

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision: str = '5116ec9ce6b0'
down_revision: Union[str, None] = '3a20784556ea'
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    pass
    # ### end Alembic commands ###


def downgrade() -> None:
    # ### commands auto generated by Alembic - please adjust! ###
    pass
    # ### end Alembic commands ###

```


## ../../arcan/__init__.py

```python
from typer import Typer, echo

cli = Typer()

__version__ = "0.1.1"


def get_arcan_version():
    try:
        import arcan

        return arcan.__version__
    except Exception as e:
        print(e)
        return "No arcan package is installed"


@cli.callback()
def callback():
    """
    Arcan AI CLI
    """


@cli.command()
def status():
    message = "Arcan is running"
    echo(message)
    return {"message": message}


@cli.command()
def version():
    message = f"Arcan version {get_arcan_version()} is installed"
    echo(message)
    return {"message": message}


def url_text_scrapping_chain(query: str, url: str) -> tuple[str, list[str]]:
    from arcan.casters.ai.chains import ArcanConversationChain
    from arcan.spells.scrapping import url_text_scrapper
    from arcan.spells.vector_search import (faiss_text_index_loader,
                                            load_faiss_vectorstore)

    chain = ArcanConversationChain()
    docsearch = None
    job_domain = None
    print(docsearch, job_domain)
    text, current_domain = url_text_scrapper(url)
    if not docsearch and current_domain != job_domain:
        try:
            print("Loading index")
            job_domain = current_domain
            docsearch = load_faiss_vectorstore(index_key=current_domain)
        except Exception as e:
            print(f"Error loading index: {e}, creating new index")
            docsearch = faiss_text_index_loader(text=text, index_key=current_domain)
    print("Running chain")
    return chain.run(query, docsearch)


# @api.get("/api/text-chat")
# @requires_auth
@cli.command()
def chat_chain(
    query: str,
    context_url: str,
    # token: HTTPAuthorizationCredentials = Depends(auth_scheme),
):
    # answer = StreamingResponse(url_text_scrapping_chain(query=query, url=context_url), media_type="text/event-stream")
    answer = url_text_scrapping_chain(query=query, url=context_url)
    return {
        "answer": answer,
    }


# @api.get("/api/arcan-chat")
# @requires_auth
@cli.command()
async def chat_agent(
    query: str,
    # token: HTTPAuthorizationCredentials = Depends(auth_scheme),
):
    from arcan.casters.ai.agents import ArcanConversationAgent, agent_chat

    agent = ArcanConversationAgent().agent
    return await agent_chat(query, agent)

```


## ../../arcan/casters/observer.py

```python
class Observer:
    def update(self, message: str):
        pass

class CasterObserver(Observer):
    def update(self, message: str):
        print(f"Caster Observer: {message}")

class Subject:
    def __init__(self):
        self._observers = []

    def attach(self, observer: Observer):
        self._observers.append(observer)

    def detach(self, observer: Observer):
        self._observers.remove(observer)

    def notify(self, message: str):
        for observer in self._observers:
            observer.update(message)

# Usage
subject = Subject()
observer = CasterObserver()
subject.attach(observer)
subject.notify("Caster state changed")

```


## ../../arcan/casters/decorators.py

```python
def log_execution(func):
    def wrapper(*args, **kwargs):
        print(f"Executing {func.__name__}")
        result = func(*args, **kwargs)
        print(f"Executed {func.__name__}")
        return result
    return wrapper

@log_execution
def some_function():
    print("Function logic here")

some_function()

```


## ../../arcan/casters/ai/interface/app.py

```python


from typing import Optional

import chainlit as cl
from langchain.schema.runnable.config import RunnableConfig
from langserve import RemoteRunnable


@cl.password_auth_callback
def auth_callback(
    username: str = "guest", password: str = "guest"
) -> Optional[cl.User]:
    # Fetch the user matching username from your database
    # and compare the hashed password with the value stored in the database
    import hashlib

    # Create a new sha256 hash object
    hash_object = hashlib.sha256()

    # Hash the password
    hash_object.update(password.encode())

    # Get the hexadecimal representation of the hash
    hashed_password = hash_object.hexdigest()

    if (username, hashed_password) == (
        "broomva",
        "b68cacbadaee450b8a8ce2dd44842f1de03ee9993ad97b5e99dea64ef93960ba",
    ):
        return cl.User(
            identifier="broomva", metadata={"role": "admin", "provider": "credentials"}
        )
    elif (username, password) == ("guest", "guest"):
        return cl.User(
            identifier="guest", metadata={"role": "user", "provider": "credentials"}
        )
    else:
        return None


def get_runnable():
    from langserve import RemoteRunnable

    spells_runnable = RemoteRunnable("https://api.arcanai.tech/spells/", headers={"arcanai_api_key": '1234'})
    return spells_runnable


# response = spells_runnable.invoke({"input": "hi there, whats my name?"},config={
#         "configurable": {"user_id": "broomva"},
#     })
# response




@cl.on_message
async def on_msg(msg: cl.Message):
    res = await get_runnable().ainvoke(
        {"input": msg.content,},
        config={"configurable": {"user_id": "broomva"},}
    )
    await cl.Message(content=res['output']).send()

    


# @cl.on_message
# async def on_msg(msg: cl.Message):
#     msg = cl.Message(content="")
        
#     async for chunk in get_runnable().astream(
#         {"input": msg.content, "chat_history": []},
#         # config=RunnableConfig(callbacks=[cl.LangchainCallbackHandler()]),
#     ):
#         await msg.stream_token(chunk['output'])

#     await msg.send()

# @cl.on_message
# async def main(message: cl.Message):
#     agent = get_runnable()
#     res = await agent.ainvoke(
#         message.content
#     )
#     await cl.Message(content=res).send()
```


## ../../arcan/casters/ai/tools/__init__.py

```python
# %%
from dotenv import load_dotenv
from langchain.agents import Tool, tool
from langchain_community.tools import WikipediaQueryRun
from langchain_community.tools.tavily_search import TavilySearchResults
from langchain_community.utilities import WikipediaAPIWrapper
from langchain_core.utils.function_calling import convert_to_openai_function
from langchain_experimental.utilities import PythonREPL

from arcan.spells.scrapping import (
    firecrawl_scrape,
    scrape_website,
    scrape_website_selenium,
)
from arcan.spells.search import serper_api_search

load_dotenv()


@tool
def get_word_length(word: str) -> int:
    """
    Returns the length of a word.

    Parameters:
    word (str): The word to calculate the length of.

    Returns:
    int: The length of the word.
    """
    return len(word)


wikipedia_tool = WikipediaQueryRun(
    api_wrapper=WikipediaAPIWrapper(top_k_results=3, doc_content_chars_max=4096)
)

tavily_tool = TavilySearchResults()

serper_api_search_tool = Tool(
    name="serper_api_search",
    func=serper_api_search,
    description="Useful for when you need to answer questions about current events, data. You should ask targeted questions. Prefer Tavily seach tool over this one",
)

scrape_with_bs4_tool = Tool(
    name="scrape_website_with_beautifulsoup",
    func=scrape_website,
    description="Useful when you need to get data from a website url; DO NOT make up any url, the url should only be from the search results. Prefer Tavily seach tool over this one unless explicitly asked to perform a scrapping task. Prefer Selenium tool and if it does not work, then use this one.",
)

scrape_with_selenuim_tool = Tool(
    name="scrape_website_with_selenium",
    func=scrape_website_selenium,
    description="Useful when you need to get data from a website url and the regular Scrape Website method is not working correctly; DO NOT make up any url, the url should only be from the search results. Prefer Tavily seach tool over this one unless explicitly asked to perform a scrapping task",
)

firecrawl_tool = Tool(
    name="firecrawl",
    func=firecrawl_scrape,
    description="Useful when you need to get data from a website url; DO NOT make up any url, use the one provided by the user.",
)

python_repl = PythonREPL()

repl_tool = Tool(
    name="python_repl",
    description="A Python shell. Use this to execute python commands. Input should be a valid python command. If you want to see the output of a value, you should print it out with `print(...)`.",
    func=python_repl.run,
)

tools = [
    get_word_length,
    wikipedia_tool,
    tavily_tool,
    # serper_api_search_tool,
    # scrape_with_bs4_tool,
    # scrape_with_selenuim_tool,
    repl_tool,
]

# %%

```


## ../../arcan/casters/ai/llm/__init__.py

```python
# %%

import os
from typing import Any, Callable, Dict, List, Optional, Union

from langchain_community.chat_models import ChatOllama
from langchain_groq import ChatGroq
from langchain_openai import ChatOpenAI, OpenAI
from pydantic import BaseModel


class LLM(BaseModel):
    """Represents a Language Learning Model (LLM) configuration and its interaction logic.

    Attributes:
        provider: A string indicating the LLM provider.
        llm: An instance of the LLM, which can be `ChatOpenAI`, `OpenAI`, or other compatible types.
        messages: A list of messages to be used for chat completions.
    """

    provider: str = "ChatOpenAI"
    llm: Optional[Union[ChatOpenAI, OpenAI]] = None
    messages: List[Dict[str, str]] = [
        {
            "role": "system",
            "content": "You are a helpful and friendly assistant.",
        }
    ]

    def __init__(self, **data: Any):
        super().__init__(**data)
        # Prevent passing 'provider' twice by excluding it from **data when calling create_llm
        llm_kwargs = {k: v for k, v in data.items() if k != "provider"}
        self.llm = LLMFactory.create_llm(self.provider, **llm_kwargs)

    class Config:
        arbitrary_types_allowed = True


class LLMFactory:
    """A factory for creating LLM instances based on the provider."""

    provider_map: Dict[str, Callable[..., Union[ChatOpenAI, OpenAI]]] = {
        "ChatOpenAI": lambda **kwargs: ChatOpenAI(
            temperature=kwargs.get("temperature", 0.7),
            model_name=kwargs.get(
                "model", os.getenv("OPENAI_MODEL", "gpt-3.5-turbo-0125")
            ),
        ),
        "ChatTogetherAI": lambda **kwargs: ChatOpenAI(
            temperature=kwargs.get("temperature", 0.7),
            model_name=kwargs.get(
                "model",
                os.getenv(
                    "TOGETHER_MODEL_NAME", "mistralai/Mixtral-8x7B-Instruct-v0.1"
                ),
            ),
            openai_api_key=kwargs.get(
                "openai_api_key", os.environ.get("TOGETHER_API_KEY")
            ),
            openai_api_base=kwargs.get(
                "openai_api_base",
                os.getenv("OPENAI_API_BASE_URL", "https://api.together.xyz/v1"),
            ),
        ),
        "ChatGroq": lambda **kwargs: ChatGroq(
            temperature=kwargs.get("temperature", 0.7),
            model_name=kwargs.get(
                "model",
                os.getenv("TOGETHER_MODEL_NAME", "llama3-8b-8192"),
            ),
        ),
        "ChatOllama": lambda **kwargs: ChatOllama(
            model=kwargs.get("model", os.getenv("OLLAMA_MODEL", "phi3")),
        ),
    }

    @staticmethod
    def create_llm(provider: str, **kwargs: Any) -> Union[ChatOpenAI, OpenAI]:
        """Creates an LLM instance based on the specified provider.

        Args:
            provider: The name of the provider.
            **kwargs: Additional keyword arguments for the provider's constructor.

        Returns:
            An instance of the specified LLM provider.

        Raises:
            NotImplementedError: If the provider is not supported.
        """
        if provider not in LLMFactory.provider_map:
            raise NotImplementedError(f"LLM provider '{provider}' not implemented.")
        return LLMFactory.provider_map[provider](**kwargs)

```


## ../../arcan/casters/ai/agents/session.py

```python
# %%

import ast
import os
import pickle
import weakref
from datetime import datetime
from typing import Any, Dict

from sqlalchemy.dialects.postgresql import insert
from sqlalchemy.orm import Session, joinedload

from arcan.forge.database.session import sessionmanager
from arcan.forge.models.chat_history import ChatHistory
from arcan.forge.models.conversation import Conversation


class ArcanSession:
    def __init__(self, database: callable = sessionmanager):
        self.database = database
        self.database_uri = os.environ.get("DATABASE_URL")
        self.agents: Dict[str, weakref.ref] = weakref.WeakValueDictionary()

    def _get_session(self) -> Session:
        if self.database is None:
            raise ValueError("Database factory is not initialized.")
        return self.database()

    def store_message(self, user_id: str, body: str, response: str):
        with self._get_session() as db_session:
            conversation = Conversation(user_id=user_id, message=body, response=response)
            db_session.add(conversation)
            db_session.commit()
            print(f"Conversation #{conversation.id} stored in database")

    def store_chat_history(self, user_id, agent_history, access_token):
        history = pickle.dumps(agent_history)
        stmt = (
            insert(ChatHistory)
            .values(
                user_id=user_id,
                access_token=access_token,
                history=str(history),
                updated_at=datetime.utcnow(),
            )
            .on_conflict_do_update(
                index_elements=["user_id"],
                set_={
                    "history": str(history),
                    "updated_at": datetime.utcnow(),
                },
            )
        )
        with self._get_session() as db:
            db.execute(stmt)
            db.commit()
            print(f"Upsert chat history for user {user_id} with statement {stmt}")

    def get_chat_history(self, user_id: str, access_token: str) -> list:
        # access token comes in hashed. Need to decode it to filter the ChatHistory table
        
        # token = login_for_access_token(user_id)
        access_token = access_token.decode("utf-8")
        
        with self._get_session() as db_session:
            history = (
                db_session.query(ChatHistory)
                # .options(joinedload(ChatHistory.history))
                .filter(ChatHistory.user_id == user_id and ChatHistory.access_token == access_token)
                .order_by(ChatHistory.updated_at.asc())
                .all()
            ) or []
        if not history:
            return []
        chat_history = history[0].history
        loaded = pickle.loads(ast.literal_eval(chat_history))
        return loaded

    def rollback(self):
        with self._get_session() as db:
            db.rollback()
            print("Rollback transaction")


# class ArcanSession:
#     def __init__(self, database: Session = None):
#         """
#         Initializes a new instance of the ArcanSession class.

#         :param database: A callable that returns a new SQLAlchemy Session instance when called.
#         """
#         self.database = database
#         self.database_uri = os.environ.get("DATABASE_URL")
#         self.agents: Dict[str, weakref.ref] = weakref.WeakValueDictionary()

# def store_message(self, user_id: str, body: str, response: str):
#     """
#     Stores a message in the database.

#     :param user_id: The unique identifier for the user.
#     :param Body: The body of the message sent by the user.
#     :param response: The response generated by the system.
#     """
#     with self.database as db_session:
#         conversation = Conversation(user_id=user_id, message=body, response=response)
#         db_session.add(conversation)
#         db_session.commit()
#         print(f"Conversation #{conversation.id} stored in database")

#     def store_chat_history(self, user_id, agent_history):
#         """
#         Stores or updates the chat history for a user in the database.

#         :param user_id: The unique identifier for the user.
#         :param agent_history: The chat history to be stored.
#         """
#         history = pickle.dumps(agent_history)
#         # Upsert statement
#         stmt = (
#             insert(ChatHistory)
#             .values(
#                 user_id=user_id,
#                 history=str(history),
#                 updated_at=datetime.utcnow(),  # Explicitly set updated_at on insert
#             )
#             .on_conflict_do_update(
#                 index_elements=["user_id"],  # Specify the conflict target
#                 set_={
#                     "history": str(history),  # Update the history field upon conflict
#                     "updated_at": datetime.utcnow(),  # Update the updated_at field upon conflict
#                 },
#             )
#         )
#         # Execute the upsert
#         with self.database as db:
#             db.execute(stmt)
#             db.commit()
#             print(f"Upsert chat history for user {user_id} with statement {stmt}")

#     def get_chat_history(self, user_id: str) -> list:
#         """
#         Retrieves the chat history for a user from the database.

#         :param db_session: The SQLAlchemy Session instance.
#         :param user_id: The unique identifier for the user.
#         :return: A list representing the chat history.
#         """
#         with self.database as db_session:
#             history = (
#                 db_session.query(ChatHistory)
#                 .filter(ChatHistory.user_id == user_id)
#                 .order_by(ChatHistory.updated_at.asc())
#                 .all()
#             ) or []
#         if not history:
#             return []
#         chat_history = history[0].history
#         loaded = pickle.loads(ast.literal_eval(chat_history))
#         return loaded
# %%

```


## ../../arcan/casters/ai/agents/__init__.py

```python
# %%
# %%
from __future__ import annotations

import ast
import asyncio
import os
import pickle
import weakref
from datetime import datetime
# Ensure necessary imports for ArcanAgent
from tempfile import TemporaryDirectory
from typing import Any, AsyncIterator, Dict, List, Optional, cast

from fastapi import Depends
from fastapi.responses import StreamingResponse
from langchain.agents import (AgentExecutor, AgentType,
                              create_tool_calling_agent, initialize_agent,
                              load_tools)
from langchain.agents.agent_types import AgentType
from langchain.agents.format_scratchpad.openai_tools import \
    format_to_openai_tool_messages
from langchain.agents.format_scratchpad.tools import format_to_tool_messages
from langchain.agents.output_parsers.tools import ToolsAgentOutputParser
from langchain.embeddings.openai import OpenAIEmbeddings
from langchain.memory import ConversationBufferMemory
from langchain.pydantic_v1 import BaseModel
from langchain_core.callbacks import CallbackManagerForChainRun
from langchain_core.messages import AIMessage, HumanMessage
from langchain_core.prompts import ChatPromptTemplate
# from langchain_core.pydantic_v1 import BaseModel
from langchain_core.runnables import (ConfigurableField, ConfigurableFieldSpec,
                                      Runnable, RunnableConfig,
                                      RunnablePassthrough,
                                      RunnableSerializable)
from langchain_core.runnables.base import Runnable, RunnableBindingBase
from langchain_core.runnables.utils import (AddableDict, AnyConfigurableField,
                                            ConfigurableField,
                                            ConfigurableFieldSpec, Input,
                                            Output, create_model,
                                            get_unique_config_specs)
from langchain_openai import ChatOpenAI, OpenAIEmbeddings
from pydantic import BaseModel, Field
from sqlalchemy import Column, ForeignKey, Integer, String
from sqlalchemy.exc import SQLAlchemyError
from sqlalchemy.orm import relationship

from arcan.casters.ai.agents.helpers import AsyncIteratorCallbackHandler
from arcan.casters.ai.agents.session import ArcanSession
from arcan.casters.ai.llm import LLM
from arcan.casters.ai.parser import ArcanOutputParser
from arcan.casters.ai.prompts import spells_agent_prompt
# from arcan.casters.ai.router import semantic_layer
from arcan.casters.ai.tools import tools as spells
from arcan.forge.schemas import Token


class ArcanAgent(RunnableSerializable):
    tools: List = Field(default_factory=list)
    bare_tools: List = Field(default_factory=list)
    agent_tools: List = Field(default_factory=list)
    agent_type: str = "arcan_spells_agent"
    chat_history: List = Field(default_factory=list)
    user_id: Optional[str] = None
    access_token: Optional[Token] = None
    verbose: bool = False
    prompt: ChatPromptTemplate = (spells_agent_prompt,)
    configs: List[ConfigurableFieldSpec] = Field(default_factory=list)
    llm_with_tools: LLM = Field(default_factory=lambda: LLM().llm)
    agent: Runnable = Field(default_factory=RunnablePassthrough)
    runnable: Runnable = Field(default_factory=RunnablePassthrough)
    session: ArcanSession = Field(default_factory=ArcanSession)

    class Config:
        arbitrary_types_allowed = True
        extra = "allow"  # This allows additional fields not explicitly defined

    def __init__(
        self,
        llm=None,
        tools: list = spells,
        prompt: ChatPromptTemplate = spells_agent_prompt,
        agent_type="arcan_spells_agent",
        chat_history: list = [],
        user_id: str = None,
        access_token: Token = None,
        verbose: bool = False,
        configs: list = [],
        **kwargs,
    ):
        super().__init__(
            tools=tools,
            agent_type=agent_type,
            chat_history=chat_history,
            user_id=user_id,
            access_token=access_token,
            verbose=verbose,
            prompt=prompt,
            configs=configs,
            **kwargs,
        )
        object.__setattr__(self, "_llm", llm or LLM().llm)
        # Initialize other fields after the main Pydantic initialization
        self.session: ArcanSession = ArcanSession()
        self.bare_tools = load_tools(["llm-math"], llm=self.llm)
        self.agent_tools = self.tools + self.bare_tools
        self.llm_with_tools = self.llm.bind_tools(self.agent_tools)
        self.agent, self.runnable = self.get_or_create_agent(self.user_id)

    @property
    def llm(self):
        return self._llm

    @property
    def default_configs(self):
        return [
            ConfigurableFieldSpec(
                id="user_id",
                annotation=str,
                name="User ID",
                description="Unique identifier for the user.",
                default="",
                is_shared=True,
            ),
            ConfigurableFieldSpec(
                id="conversation_id",
                annotation=str,
                name="Conversation ID",
                description="Unique identifier for the conversation.",
                default="",
                is_shared=True,
            ),
            ConfigurableFieldSpec(
                id="access_token",
                annotation=Token,
                name="Access Token",
                description="The access token for the user.",
                default=None,
                is_shared=True,
            ),
        ]

    @property
    def config_specs(self) -> List[ConfigurableFieldSpec]:
        return self.agent.config_specs

    async def astream(
        self,
        input: Input,
        config: Optional[RunnableConfig] = None,
        **kwargs: Optional[Any],
    ) -> AsyncIterator[Output]:
        """Stream the agent's output."""
        configurable = cast(Dict[str, Any], config.pop("configurable", {}))

        if configurable:
            configured_agent = self.agent.with_config(
                {
                    "configurable": configurable,
                }
            )
        else:
            configured_agent = self.agent

        self.runnable.with_config({"run_name": "executor"})

        async for output in self.runnable.astream(input, config=config, **kwargs):
            yield output

    def get_or_create_agent(
        self, user_id: str, access_token: Token = None, provided_agent: ArcanAgent = None
    ) -> ArcanAgent:
        """
        Retrieves or creates a ArcanAgent for a given user_id.

        :param user_id: The unique identifier for the user.
        :param access_token: The access token for the user.
        :param provided_agent: An existing agent to use.
        
        :return: An instance of ArcanAgent.
        """
        if provided_agent is None:
            agent = self.session.agents.get(user_id)
            chat_history = []

            # Obtain a new database session
            try:
                chat_history = self.session.get_chat_history(user_id=user_id, access_token=access_token.access_token)
            except Exception as e:
                print(f"Error getting chat history for {user_id}: {e}")

            if agent is not None and chat_history:
                print(f"Using existing agent {agent}")
            elif agent is None and chat_history:
                print(f"Using reloaded agent with history {chat_history}")
                self.chat_history = chat_history
            elif agent is None and not chat_history:
                print("Using a new agent")
            agent, runnable = self.get_agent()
            self.session.agents[user_id] = agent
            return agent, runnable
        else:
            provided_agent.user_id = user_id
            self.session.agents[user_id] = provided_agent
            return provided_agent, provided_agent.runnable

    def get_agent(self):
        """
        Retrieves or creates a ArcanAgent for a given user_id.

        :param user_id: The unique identifier for the user.
        :return: An instance of ArcanAgent.
        """
        if self.session is None:
            raise ValueError("Session is not initialized.")
        agent = (
            RunnablePassthrough.assign(
                agent_scratchpad=lambda x: format_to_tool_messages(
                    x["intermediate_steps"]
                )
            )
            | self.prompt
            | self.llm_with_tools
            | ToolsAgentOutputParser()
        )
        runnable = AgentExecutor(
            agent=agent, tools=self.agent_tools, verbose=self.verbose
        )
        return agent, runnable

    def invoke(
        self,
        inputs: Dict[str, Any],
        run_manager: Optional[CallbackManagerForChainRun] = None,
    ) -> Dict[str, Any]:
        """
        Override the invoke method to include custom logic.
        """
        user_content = inputs.get("input")
        if not user_content:
            raise ValueError("Input must contain 'input' key with user content.")

        # route_text, routed_content = semantic_layer(
        #     query=user_content, user_id=self.user_id
        # )
        self.chat_history.extend(
            [
                # SystemMessage(content=route_text),
                HumanMessage(content=user_content),
            ]
        )
        response = self.runnable.invoke(
            {"input": user_content, "chat_history": self.chat_history}
        )
        self.chat_history.extend(
            [
                AIMessage(content=response["output"]),
            ]
        )
        try:
            self.session.store_message(
                user_id=self.user_id, body=user_content, response=response["output"]
            )
            self.session.store_chat_history(
                user_id=self.user_id, agent_history=self.chat_history, access_token=self.access_token
            )
        except SQLAlchemyError as e:
            self.session.database.rollback()
            print(f"Error storing conversation in database: {e}")
        return response








    # def configurable_fields(
    #     self, **kwargs: AnyConfigurableField
    # ):
    #     """Configure particular runnable fields at runtime.

    #     .. code-block:: python

    #         from langchain_core.runnables import ConfigurableField
    #         from langchain_openai import ChatOpenAI

    #         model = ChatOpenAI(max_tokens=20).configurable_fields(
    #             max_tokens=ConfigurableField(
    #                 id="output_token_number",
    #                 name="Max tokens in the output",
    #                 description="The maximum number of tokens in the output",
    #             )
    #         )

    #         # max_tokens = 20
    #         print(
    #             "max_tokens_20: ",
    #             model.invoke("tell me something about chess").content
    #         )

    #         # max_tokens = 200
    #         print("max_tokens_200: ", model.with_config(
    #             configurable={"output_token_number": 200}
    #             ).invoke("tell me something about chess").content
    #         )
    #     """
    #     from langchain_core.runnables.configurable import \
    #         RunnableConfigurableFields

    #     for key in kwargs:
    #         # print(f"Checking key {key} in {self}")
    #         # print(f"Available keys are {vars(self).keys()}")
    #         if key not in vars(self).keys():
    #             raise ValueError(
    #                 f"Configuration key {key} not found in {self}: "
    #                 f"available keys are {vars(self).keys()}"
    #             )
    #         # updated the self class arguments with the new values
    #         setattr(self, key, kwargs[key])
    #     return self

    # %%

    # class ArcanAgent(RunnableSerializable):
    #     """
    #     Represents an Arcan Agent that interacts with the user and provides responses using OpenAI tools.

    #     Attributes:
    #         llm (LLM): The Language Model Manager used by the agent.
    #         tools (list): The list of tools used by the agent.
    #         hub_prompt (str): The prompt for the OpenAI tools agent.
    #         agent_type (str): The type of the agent.
    #         chat_history (list): The chat history of the agent.
    #         llm_with_tools: The Language Model Manager with the tools bound.
    #         prompt: The chat prompt template for the agent.
    #         agent: The agent pipeline.
    #         agent_executor: The executor for the agent.
    #         user_id: The unique identifier for the user.
    #         verbose: A boolean indicating whether to print verbose output.
    #     """
    #     llm: LLM = LLM().llm
    #     tools: List = spells
    #     agent_type: str = 'arcan_spells_agent'
    #     chat_history: List = Field(default_factory=list)
    #     user_id: Optional[str] = None
    #     verbose: bool = False
    #     # Assuming session and prompt types are defined somewhere
    #     session: ArcanSession
    #     prompt: str = spells_agent_prompt
    #     configs: List[ConfigurableFieldSpec] = Field(default_factory=list)

    #     class Config:
    #         arbitrary_types_allowed = True

    #     def __init__(self, llm: LLM = LLM().llm, tools: list = spells, prompt: str = spells_agent_prompt,
    #                  agent_type="arcan_spells_agent", chat_history: list = [], user_id: str = None,
    #                  verbose: bool = False, configs: list = None, **kwargs):
    #         super().__init__(**kwargs)  # Initialize BaseModel with kwargs
    #         self.llm = llm
    #         self.tools = tools
    #         self.agent_type = agent_type
    #         self.chat_history = chat_history
    #         self.user_id = user_id
    #         self.verbose = verbose
    #         self.session = ArcanSession()
    #         self.prompt = prompt
    #         self.working_directory = TemporaryDirectory()
    #         self.file_system_tools = FileManagementToolkit(
    #             root_dir=str(self.working_directory.name)
    #         ).get_tools()
    #         self.bare_tools = load_tools(
    #             [
    #                 "llm-math",
    #             ],
    #             llm=self.llm,
    #         )
    #         self.agent_tools = self.tools + self.bare_tools
    #         self.llm_with_tools = self.llm.bind_tools(self.agent_tools)
    #         missing_vars = {"agent_scratchpad"}.difference(
    #             prompt.input_variables + list(prompt.partial_variables)
    #         )
    #         if missing_vars:
    #             raise ValueError(f"Prompt missing required variables: {missing_vars}")

    #         if not hasattr(llm, "bind_tools"):
    #             raise ValueError(
    #                 "This function requires a .bind_tools method be implemented on the LLM.",
    #             )
    #         self.llm_with_tools = llm.bind_tools(tools)

    #         self.agent, self.runnable = self.get_or_create_agent(self.user_id)

    #         self.configs = configs or [
    #             ConfigurableFieldSpec(
    #                 id="user_id",
    #                 annotation=str,
    #                 name="User ID",
    #                 description="Unique identifier for the user.",
    #                 default="",
    #                 is_shared=True,
    #             ),
    #             ConfigurableFieldSpec(
    #                 id="conversation_id",
    #                 annotation=str,
    #                 name="Conversation ID",
    #                 description="Unique identifier for the conversation.",
    #                 default="",
    #                 is_shared=True,
    #             )
    #         ]

    # def __init__(
    #     self,
    #     llm: LLM = LLM().llm,
    #     tools: list = spells,
    #     prompt: str = spells_agent_prompt,
    #     agent_type="arcan_spells_agent",
    #     chat_history: list = [],  # represents the chat history, can be pulled from a db
    #     user_id: str = None,
    #     verbose: bool = False,
    #     session_factory: callable = session_scope,
    #     configs: list = [],
    #     **kwargs
    #     ):
    #     """Initialize the runnable."""
    #     super().__init__(**kwargs)
    #     self.llm: LLM = llm
    #     self.tools: list = tools
    #     self.agent_type: str = agent_type
    #     self.chat_history: list = chat_history
    #     self.user_id: str = kwargs.get('user_id', user_id)
    #     self.verbose: bool = verbose
    #     self.session: ArcanSession = ArcanSession()
    #     self.prompt = prompt
    #     self.working_directory = TemporaryDirectory()
    #     self.file_system_tools = FileManagementToolkit(
    #         root_dir=str(self.working_directory.name)
    #     ).get_tools()
    #     self.bare_tools = load_tools(
    #         [
    #             "llm-math",
    #         ],
    #         llm=self.llm,
    #     )
    #     self.agent_tools = self.tools + self.bare_tools
    #     self.llm_with_tools = self.llm.bind_tools(self.agent_tools)
    #     missing_vars = {"agent_scratchpad"}.difference(
    #         prompt.input_variables + list(prompt.partial_variables)
    #     )
    #     if missing_vars:
    #         raise ValueError(f"Prompt missing required variables: {missing_vars}")

    #     if not hasattr(llm, "bind_tools"):
    #         raise ValueError(
    #             "This function requires a .bind_tools method be implemented on the LLM.",
    #         )
    #     self.llm_with_tools = llm.bind_tools(tools)

    #     self.agent, self.runnable = self.get_or_create_agent(self.user_id)

    #     self.configs = configs if configs is not None else [
    #         ConfigurableFieldSpec(
    #             id="user_id",
    #             annotation=str,
    #             name="User ID",
    #             description="Unique identifier for the user.",
    #             default="",
    #             is_shared=True,
    #         ),
    #         ConfigurableFieldSpec(
    #             id="conversation_id",
    #             annotation=str,
    #             name="Conversation ID",
    #             description="Unique identifier for the conversation.",
    #             default="",
    #             is_shared=True,
    #         )
    #     ]

    # @property
    # def config_specs(self) -> List[ConfigurableFieldSpec]:
    #     return self.agent.config_specs

    # async def astream(
    #     self,
    #     input: Input,
    #     config: Optional[RunnableConfig] = None,
    #     **kwargs: Optional[Any],
    # ) -> AsyncIterator[Output]:
    #     """Stream the agent's output."""
    #     configurable = cast(Dict[str, Any], config.pop("configurable", {}))

    #     if configurable:
    #         configured_agent = self.agent.with_config(
    #             {
    #                 "configurable": configurable,
    #             }
    #         )
    #     else:
    #         configured_agent = self.agent

    #     self.runnable.with_config({"run_name": "executor"})

    #     async for output in self.runnable.astream(input, config=config, **kwargs):
    #         yield output

    # def get_or_create_agent(
    #     self, user_id: str, provided_agent: ArcanAgent = None
    # ) -> ArcanAgent:
    #     """
    #     Retrieves or creates a ArcanAgent for a given user_id.

    #     :param user_id: The unique identifier for the user.
    #     :return: An instance of ArcanAgent.
    #     """
    #     if provided_agent is None:
    #         agent = self.session.agents.get(user_id)
    #         chat_history = []

    #         # Obtain a new database session
    #         try:
    #             chat_history = self.session.get_chat_history(user_id)
    #         except Exception as e:
    #             print(f"Error getting chat history for {user_id}: {e}")

    #         if agent is not None and chat_history:
    #             print(f"Using existing agent {agent}")
    #         elif agent is None and chat_history:
    #             print(f"Using reloaded agent with history {chat_history}")
    #             self.chat_history = chat_history
    #         elif agent is None and not chat_history:
    #             print("Using a new agent")
    #         agent, runnable = self.get_agent()
    #         self.session.agents[user_id] = agent
    #         return agent, runnable
    #     else:
    #         provided_agent.user_id = user_id
    #         self.session.agents[user_id] = provided_agent
    #         return provided_agent, provided_agent.runnable

    # def get_agent(self):
    #     """
    #     Retrieves or creates a ArcanAgent for a given user_id.

    #     :param user_id: The unique identifier for the user.
    #     :return: An instance of ArcanAgent.
    #     """
    #     if self.session is None:
    #         raise ValueError("Session is not initialized.")
    #     agent = (
    #         RunnablePassthrough.assign(
    #             agent_scratchpad=lambda x: format_to_tool_messages(
    #                 x["intermediate_steps"]
    #             )
    #         )
    #         | self.prompt
    #         | self.llm_with_tools
    #         | ToolsAgentOutputParser()
    #     )
    #     runnable = AgentExecutor(
    #         agent=agent, tools=self.agent_tools, verbose=self.verbose
    #     )
    #     return agent, runnable

    # def invoke(
    #     self,
    #     inputs: Dict[str, Any],
    #     run_manager: Optional[CallbackManagerForChainRun] = None,
    # ) -> Dict[str, Any]:
    #     """
    #     Override the invoke method to include custom logic.
    #     """
    #     user_content = inputs.get("input")
    #     if not user_content:
    #         raise ValueError("Input must contain 'input' key with user content.")

    #     # route_text, routed_content = semantic_layer(
    #     #     query=user_content, user_id=self.user_id
    #     # )
    #     self.chat_history.extend(
    #         [
    #             # SystemMessage(content=route_text),
    #             HumanMessage(content=user_content),
    #         ]
    #     )
    #     response = self.runnable.invoke(
    #         {"input": user_content, "chat_history": self.chat_history}
    #     )
    #     self.chat_history.extend(
    #         [
    #             AIMessage(content=response["output"]),
    #         ]
    #     )
    #     # try:
    #     #     self.session.store_message(user_id=self.user_id, body=user_content, response=response)
    #     #     self.session.store_chat_history(user_id=self.user_id, agent_history=self.chat_history)
    #     # except SQLAlchemyError as e:
    #     #     self.session.database.rollback()
    #     #     print(f"Error storing conversation in database: {e}")
    #     return response

    # def configurable_fields(self, **kwargs: AnyConfigurableField):
    #     """Configure particular runnable fields at runtime.

    #     .. code-block:: python

    #         from langchain_core.runnables import ConfigurableField
    #         from langchain_openai import ChatOpenAI

    #         model = ChatOpenAI(max_tokens=20).configurable_fields(
    #             max_tokens=ConfigurableField(
    #                 id="output_token_number",
    #                 name="Max tokens in the output",
    #                 description="The maximum number of tokens in the output",
    #             )
    #         )

    #         # max_tokens = 20
    #         print(
    #             "max_tokens_20: ",
    #             model.invoke("tell me something about chess").content
    #         )

    #         # max_tokens = 200
    #         print("max_tokens_200: ", model.with_config(
    #             configurable={"output_token_number": 200}
    #             ).invoke("tell me something about chess").content
    #         )
    #     """
    #     from langchain_core.runnables.configurable import \
    #         RunnableConfigurableFields

    #     for key in kwargs:
    #         # print(f"Checking key {key} in {self}")
    #         # print(f"Available keys are {vars(self).keys()}")
    #         if key not in vars(self).keys():
    #             raise ValueError(
    #                 f"Configuration key {key} not found in {self}: "
    #                 f"available keys are {vars(self).keys()}"
    #             )
    #         # updated the self class arguments with the new values
    #         setattr(self, key, kwargs[key])
    #     return self


# %%

# %%


# %%


# class ArcanAgent:
#     """
#     Represents a Arcan Agent that interacts with the user and provides responses using OpenAI tools.

#     Attributes:
#         llm (LLM): The Language Model Manager used by the agent.
#         tools (list): The list of tools used by the agent.
#         hub_prompt (str): The prompt for the OpenAI tools agent.
#         agent_type (str): The type of the agent.
#         chat_history (list): The chat history of the agent.
#         llm_with_tools: The Language Model Manager with the tools bound.
#         prompt: The chat prompt template for the agent.
#         agent: The agent pipeline.
#         agent_executor: The executor for the agent.
#         user_id: The unique identifier for the user.
#         verbose: A boolean indicating whether to print verbose output.

#     Methods:
#         get_response: Gets the response from the agent given user input.

#     """

#     def __init__(
#         self,
#         # database: SQLDatabase,
#         llm: LLM = LLM().llm,
#         tools: list = spells,
#         hub_prompt: str = "broomva/arcan",
#         agent_type="arcan_spells_agent",
#         context: list = [],  # represents the chat history, can be pulled from a db
#         user_id: str = None,
#         verbose: bool = False,
#     ):
#         self.llm: LLM = llm
#         self.tools: list = tools
#         self.hub_prompt: str = hub_prompt
#         self.agent_type: str = agent_type
#         self.chat_history: list = context
#         self.user_id: str = user_id
#         self.verbose: bool = verbose

#         # self.db = database
#         # self.toolkit = SQLDatabaseToolkit(db=self.db, llm=self.llm)
#         # self.context = self.toolkit.get_context()
#         self.prompt = arcan_prompt  # .partial(**self.context)
#         # self.sql_tools = self.toolkit.get_tools()
#         self.working_directory = TemporaryDirectory()
#         self.file_system_tools = FileManagementToolkit(
#             root_dir=str(self.working_directory.name)
#         ).get_tools()
#         self.parser = ArcanOutputParser()
#         self.bare_tools = load_tools(
#             [
#                 "llm-math",
#                 # "human",
#                 # "wolfram-alpha"
#             ],
#             llm=self.llm,
#         )
#         self.agent_tools = (
#             self.tools + self.bare_tools  # + self.sql_tools + self.file_system_tools
#         )
#         self.llm_with_tools = self.llm.bind_tools(self.agent_tools)
#         self.agent = (
#             {
#                 "input": lambda x: x["input"],
#                 "agent_scratchpad": lambda x: format_to_openai_tool_messages(
#                     x["intermediate_steps"]
#                 ),
#                 "chat_history": lambda x: x["chat_history"],
#             }
#             | self.prompt
#             | self.llm_with_tools
#             | self.parser
#         )
#         self.agent_executor = AgentExecutor(
#             agent=self.agent, tools=self.agent_tools, verbose=self.verbose
#         )

#     def get_response(self, user_content: str):
#         """
#         Gets the response from the agent given user input.

#         Args:
#             user_content (str): The user input.

#         Returns:
#             str: The response from the agent.

#         """
#         # routed_content = semantic_layer(query=user_content, user_id=self.user_id)
#         response = self.agent_executor.invoke(
#             {"input": user_content, "chat_history": self.chat_history}
#         )
#         self.chat_history.extend(
#             [
#                 HumanMessage(content=user_content),
#                 AIMessage(content=response["output"]),
#             ]
#         )
#         return response["output"]


# class ArcanSpellsAgent(ArcanAgent):
#     """
#     Represents a Arcan Agent that interacts with the user and provides responses using OpenAI tools.

#     Attributes:
#         llm (LLM): The Language Model Manager used by the agent.
#         tools (list): The list of tools used by the agent.
#         hub_prompt (str): The prompt for the OpenAI tools agent.
#         agent_type (str): The type of the agent.
#         chat_history (list): The chat history of the agent.
#         llm_with_tools: The Language Model Manager with the tools bound.
#         prompt: The chat prompt template for the agent.
#         agent: The agent pipeline.
#         agent_executor: The executor for the agent.
#         user_id: The unique identifier for the user.
#         verbose: A boolean indicating whether to print verbose output.

#     Methods:
#         get_response: Gets the response from the agent given user input.

#     """

#     def __init__(
#         self,
#         # database: SQLDatabase,
#         llm: LLM = LLM().llm,
#         tools: list = spells,
#         prompt: str = spells_agent_prompt,
#         agent_type="arcan_spells_agent",
#         context: list = [],  # represents the chat history, can be pulled from a db
#         user_id: str = None,
#         verbose: bool = False,
#     ):
#         self.llm: LLM = llm
#         self.tools: list = tools
#         self.agent_type: str = agent_type
#         self.chat_history: list = context
#         self.user_id: str = user_id
#         self.verbose: bool = verbose
#         # self.database = database
#         # self.toolkit = SQLDatabaseToolkit(db=database, llm=self.llm)
#         # self.context = self.toolkit.get_context()
#         # self.sql_tools = self.toolkit.get_tools()
#         self.prompt = prompt  # arcan_prompt.partial(**self.context)
#         self.working_directory = TemporaryDirectory()
#         self.file_system_tools = FileManagementToolkit(
#             root_dir=str(self.working_directory.name)
#         ).get_tools()
#         self.parser = ToolsAgentOutputParser()
#         self.bare_tools = load_tools(
#             [
#                 "llm-math",
#                 # "human",
#                 # "wolfram-alpha"
#             ],
#             llm=self.llm,
#         )
#         self.agent_tools = (
#             self.tools + self.bare_tools  # + self.sql_tools + self.file_system_tools
#         )
#         self.llm_with_tools = self.llm.bind_tools(self.agent_tools)
#         # Construct the Tools agent
#         # self.agent = create_tool_calling_agent(self.llm, self.agent_tools, self.prompt)
#         self.agent = (
#             {
#                 "input": lambda x: x["input"],
#                 "agent_scratchpad": lambda x: format_to_openai_tool_messages(
#                     x["intermediate_steps"]
#                 ),
#                 "chat_history": lambda x: x["chat_history"],
#             }
#             | self.prompt
#             | self.llm_with_tools
#             | self.parser
#         )
#         self.agent_executor = AgentExecutor(
#             agent=self.agent, tools=self.agent_tools, verbose=self.verbose
#         )

#     def get_response(self, user_content: str):
#         """
#         Gets the response from the agent given user input.

#         Args:
#             user_content (str): The user input.

#         Returns:
#             str: The response from the agent.

#         """
#         routed_content, route_text = semantic_layer(
#             query=user_content, user_id=self.user_id
#         )
#         response = self.agent_executor.invoke(
#             {"input": routed_content, "chat_history": self.chat_history}
#         )
#         self.chat_history.extend(
#             [
#                 AIMessage(content=route_text),
#                 HumanMessage(content=user_content),
#                 AIMessage(content=response["output"]),
#             ]
#         )
#         return response["output"]


# %%


class ArcanConversationAgent:
    def __init__(self, **kwargs):
        self.kwargs = kwargs
        self.llm = LLM().llm
        self.embeddings = OpenAIEmbeddings()
        self.memory = ConversationBufferMemory(  # ConversationBufferWindowMemory k=10
            memory_key="chat_history", return_messages=True, output_key="output"
        )
        self.tools = load_tools(["llm-math"], llm=self.llm)
        self.agent = initialize_agent(
            agent=AgentType.CHAT_CONVERSATIONAL_REACT_DESCRIPTION,
            tools=self.tools,
            llm=self.llm,
            verbose=True,
            max_iterations=3,
            early_stopping_method="generate",
            memory=self.memory,
            return_intermediate_steps=True,
            agent_kwargs={"output_parser": ArcanOutputParser()},
            # output_parser=ArcanOutputParser
        )


class Query(BaseModel):
    text: str


async def run_call(query: str, stream_it: AsyncIteratorCallbackHandler, agent):
    try:
        # assign callback handler
        agent.agent.llm_chain.llm.callbacks = [stream_it]
        # now query
        await agent.acall(inputs={"input": query})
    except Exception as e:
        print(f"run_call {e}")
        raise (e)


async def create_gen(query: str, stream_it: AsyncIteratorCallbackHandler, agent):
    try:
        task = asyncio.create_task(run_call(query, stream_it, agent))
        async for token in stream_it.aiter():
            yield token
        await task
    except Exception as e:
        print(f"Error: {e}")
        yield str(e)
        raise e


async def agent_chat(text: str, agent):  # query: Query = Body(...),):
    stream_it = AsyncIteratorCallbackHandler()  # AsyncCallbackHandler()
    query = Query(text=text)
    try:
        gen = create_gen(query.text, stream_it, agent)
    except Exception as e:
        raise (e)
    return StreamingResponse(gen, media_type="text/event-stream")


# %%


# %%

```


## ../../arcan/casters/ai/agents/researcher.py

```python
import json
import os
from typing import Type

import requests
from bs4 import BeautifulSoup
from dotenv import load_dotenv
from fastapi import FastAPI
from langchain import PromptTemplate
from langchain.agents import AgentType, Tool, initialize_agent
from langchain.chains.summarize import load_summarize_chain
from langchain.chat_models import ChatOpenAI
from langchain.memory import conversationummaryBufferMemory
from langchain.prompts import MessagesPlaceholder
from langchain.schema import SystemMessage
from langchain.text_splitter import RecursiveCharacterTextSplitter
from langchain.tools import BaseTool
from pydantic import BaseModel, Field

load_dotenv()
brwoserless_api_key = os.getenv("BROWSERLESS_API_KEY")
serper_api_key = os.getenv("SERP_API_KEY")

# 1. Tool for search


def search(query):
    url = "https://google.serper.dev/search"

    payload = json.dumps({
        "q": query
    })

    headers = {
        'X-API-KEY': serper_api_key,
        'Content-Type': 'application/json'
    }

    response = requests.request("POST", url, headers=headers, data=payload)

    print(response.text)

    return response.text


# 2. Tool for scraping
def scrape_website(objective: str, url: str):
    # scrape website, and also will summarize the content based on objective if the content is too large
    # objective is the original objective & task that user give to the agent, url is the url of the website to be scraped

    print("Scraping website...")
    # Define the headers for the request
    headers = {
        'Cache-Control': 'no-cache',
        'Content-Type': 'application/json',
    }

    # Define the data to be sent in the request
    data = {
        "url": url
    }

    # Convert Python object to JSON string
    data_json = json.dumps(data)

    # Send the POST request
    post_url = f"https://chrome.browserless.io/content?token={brwoserless_api_key}"
    response = requests.post(post_url, headers=headers, data=data_json)

    # Check the response status code
    if response.status_code == 200:
        soup = BeautifulSoup(response.content, "html.parser")
        text = soup.get_text()
        print("CONTENTTTTTT:", text)

        if len(text) > 10000:
            output = summary(objective, text)
            return output
        else:
            return text
    else:
        print(f"HTTP request failed with status code {response.status_code}")


def summary(objective, content):
    llm = ChatOpenAI(temperature=0, model="gpt-3.5-turbo-16k-0613")

    text_splitter = RecursiveCharacterTextSplitter(
        separators=["\n\n", "\n"], chunk_size=10000, chunk_overlap=500)
    docs = text_splitter.create_documents([content])
    map_prompt = """
    Write a summary of the following text for {objective}:
    "{text}"
    SUMMARY:
    """
    map_prompt_template = PromptTemplate(
        template=map_prompt, input_variables=["text", "objective"])

    summary_chain = load_summarize_chain(
        llm=llm,
        chain_type='map_reduce',
        map_prompt=map_prompt_template,
        combine_prompt=map_prompt_template,
        verbose=True
    )

    output = summary_chain.run(input_documents=docs, objective=objective)

    return output


class ScrapeWebsiteInput(BaseModel):
    """Inputs for scrape_website"""
    objective: str = Field(
        description="The objective & task that users give to the agent")
    url: str = Field(description="The url of the website to be scraped")


class ScrapeWebsiteTool(BaseTool):
    name = "scrape_website"
    description = "useful when you need to get data from a website url, passing both url and objective to the function; DO NOT make up any url, the url should only be from the search results"
    args_schema: Type[BaseModel] = ScrapeWebsiteInput

    def _run(self, objective: str, url: str):
        return scrape_website(objective, url)

    def _arun(self, url: str):
        raise NotImplementedError("error here")


# 3. Create langchain agent with the tools above
tools = [
    Tool(
        name="Search",
        func=search,
        description="useful for when you need to answer questions about current events, data. You should ask targeted questions"
    ),
    ScrapeWebsiteTool(),
]

system_message = SystemMessage(
    content="""You are a world class researcher, who can do detailed research on any topic and produce facts based results; 
            you do not make things up, you will try as hard as possible to gather facts & data to back up the research
            
            Please make sure you complete the objective above with the following rules:
            1/ You should do enough research to gather as much information as possible about the objective
            2/ If there are url of relevant links & articles, you will scrape it to gather more information
            3/ After scraping & search, you should think "is there any new things i should search & scraping based on the data I collected to increase research quality?" If answer is yes, continue; But don't do this more than 3 iteratins
            4/ You should not make things up, you should only write facts & data that you have gathered
            5/ In the final output, You should include all reference data & links to back up your research; You should include all reference data & links to back up your research
            6/ In the final output, You should include all reference data & links to back up your research; You should include all reference data & links to back up your research"""
)

agent_kwargs = {
    "extra_prompt_messages": [MessagesPlaceholder(variable_name="memory")],
    "system_message": system_message,
}

llm = ChatOpenAI(temperature=0, model="gpt-3.5-turbo-16k-0613")
memory = conversationummaryBufferMemory(
    memory_key="memory", return_messages=True, llm=llm, max_token_limit=1000)

agent = initialize_agent(
    tools,
    llm,
    agent=AgentType.OPENAI_FUNCTIONS,
    verbose=True,
    agent_kwargs=agent_kwargs,
    memory=memory,
)


# 4. Use streamlit to create a web app
# def main():
#     st.set_page_config(page_title="AI research agent", page_icon=":bird:")

#     st.header("AI research agent :bird:")
#     query = st.text_input("Research goal")

#     if query:
#         st.write("Doing research for ", query)

#         result = agent({"input": query})

#         st.info(result['output'])


# if __name__ == '__main__':
#     main()


# 5. Set this as an API endpoint via FastAPI
app = FastAPI()


class Query(BaseModel):
    query: str


@app.post("/")
def researchAgent(query: Query):
    query = query.query
    content = agent({"input": query})
    actual_content = content['output']
    return actual_content
```


## ../../arcan/casters/ai/agents/helpers.py

```python
from __future__ import annotations

import asyncio
from typing import Any, AsyncIterator, Dict, List, Literal, Union, cast

import requests
from langchain.callbacks.base import AsyncCallbackHandler
from langchain.schema.output import LLMResult

# from langchain.callbacks.streaming_aiter import AsyncIteratorCallbackHandler


def get_stream_response(
    url: str = "https://chat.arcanai.tech",
    query: str = "Hi",
    headers: dict = None,
):
    session = requests.Session()
    with session.get(
        f"{url}?query={query}",
        stream=True,
        headers=headers,
    ) as response:
        for line in response.iter_content():
            print(line.decode("utf-8"), end="")


# class AsyncCallbackHandler(AsyncIteratorCallbackHandler):
#     content: str = ""
#     finished: bool = False

#     async def on_llm_new_token(self, token: str, **kwargs: Any) -> None:
#         self.content += token

#         if not self.finished and '"action": "Final Answer"' in self.content:
#             self.finished = True
#             self.content = ""

#         # If inside the "Final Answer" action, start collecting tokens for action_input.
#         elif self.finished and '"action_input": "' in self.content:
#             if token not in ['"', "}"]:
#                 self.queue.put_nowait(token)


#     async def on_llm_end(self, response: LLMResult, **kwargs: Any) -> None:
#         if self.finished:
#             self.done.set()
#             self.finished = False


class AsyncIteratorCallbackHandler(AsyncCallbackHandler):
    """Callback handler that returns an async iterator."""

    content: str = ""
    finished: bool = False
    queue: asyncio.Queue[str]
    done: asyncio.Event

    @property
    def always_verbose(self) -> bool:
        return True

    def __init__(self) -> None:
        self.queue = asyncio.Queue()
        self.done = asyncio.Event()

    async def on_llm_start(
        self, serialized: Dict[str, Any], prompts: List[str], **kwargs: Any
    ) -> None:
        # If two calls are made in a row, this resets the state
        self.done.clear()

    async def on_llm_new_token(self, token: str, **kwargs: Any) -> None:
        self.content += token

        if not self.finished and '"action": "Final Answer"' in self.content:
            self.finished = True
            self.content = ""

        # If inside the "Final Answer" action, start collecting tokens for action_input.
        elif self.finished and '"action_input": "' in self.content:
            if token not in ['"', "}"]:
                self.queue.put_nowait(token)

    async def on_llm_end(self, response: LLMResult, **kwargs: Any) -> None:
        if self.finished:
            self.done.set()
            self.finished = False

    async def on_llm_error(self, error: BaseException, **kwargs: Any) -> None:
        self.done.set()

    # TODO implement the other methods

    async def aiter(self) -> AsyncIterator[str]:
        try:
            while not self.queue.empty() or not self.done.is_set():
                # Wait for the next token in the queue,
                # but stop waiting if the done event is set
                done, other = await asyncio.wait(
                    [
                        # NOTE: If you add other tasks here, update the code below,
                        # which assumes each set has exactly one task each
                        asyncio.ensure_future(self.queue.get()),
                        asyncio.ensure_future(self.done.wait()),
                    ],
                    return_when=asyncio.FIRST_COMPLETED,
                )

                # Cancel the other task
                if other:
                    other.pop().cancel()

                # Extract the value of the first completed task
                token_or_done = cast(Union[str, Literal[True]], done.pop().result())

                # If the extracted value is the boolean True, the done event was set
                if token_or_done is True:
                    break

                # Otherwise, the extracted value is a token, which we yield
                yield token_or_done
        except Exception as e:
            print(f"aiter error {e}")
            raise e

```


## ../../arcan/casters/ai/parser/__init__.py

```python
from __future__ import annotations

from typing import Union

from langchain.agents import AgentOutputParser
# from langchain.agents.conversational_chat.prompt import FORMAT_INSTRUCTIONS
from langchain.output_parsers.json import parse_json_markdown
from langchain.schema import AgentAction, AgentFinish, OutputParserException

from arcan.casters.ai.prompts import FORMAT_INSTRUCTIONS


# Define a class that parses output for conversational agents
class ArcanOutputParser(AgentOutputParser):
    """Output parser for the conversational agent."""

    def get_format_instructions(self) -> str:
        """Returns formatting instructions for the given output parser."""
        return FORMAT_INSTRUCTIONS

    def parse(self, text: str) -> Union[AgentAction, AgentFinish]:
        """Attempts to parse the given text into an AgentAction or AgentFinish.

        Raises:
             OutputParserException if parsing fails.
        """
        try:
            # Attempt to parse the text into a structured format (assumed to be JSON
            # stored as markdown)
            response = parse_json_markdown(text)

            # If the response contains an 'action' and 'action_input'
            if "action" in response and "action_input" in response:
                action, action_input = response["action"], response["action_input"]

                # If the action indicates a final answer, return an AgentFinish
                if action == "Final Answer":
                    return AgentFinish({"output": action_input}, text)
                else:
                    # Otherwise, return an AgentAction with the specified action and
                    # input
                    return AgentAction(action, action_input, text)
            else:
                # If the necessary keys aren't present in the response, raise an
                # exception
                raise OutputParserException(
                    f"Missing 'action' or 'action_input' in LLM output: {text}"
                )
        except Exception as e:
            # If any other exception is raised during parsing, also raise an
            # OutputParserException
            # formated_output = f'{{"action": "Final Answer"\n"action_input": " {str(text)} }}"'
            # return AgentFinish({"output": formated_output}, formated_output)
            raise OutputParserException(
                f"Could not parse LLM output with default settings: {text}"
            ) from e

    @property
    def _type(self) -> str:
        return "conversational_chat"

```


## ../../arcan/casters/ai/runnables/__init__.py

```python
# %%
from langchain.agents import AgentExecutor
from langchain_core.runnables import Runnable
from langchain_groq import ChatGroq
from langchain_openai import ChatOpenAI
from langserve import RemoteRunnable


class RunnableFactory:
    def __init__(self, base_url: str = "http://localhost:8000/"):
        self.base_url = base_url
        self.runnable_cache = {}

    def get_runnable(self, runnable_name: str, cache: bool = True) -> RemoteRunnable:
        if cache and runnable_name in self.runnable_cache:
            return self.runnable_cache[runnable_name]

        runnable = RemoteRunnable(self.base_url + runnable_name + "/")
        if cache:
            self.runnable_cache[runnable_name] = runnable
        return runnable


class ArcanRunnables:
    def __init__(self, base_url: str = "http://localhost:8000/"):
        self.factory = RunnableFactory(base_url=base_url)

    def get_spells_runnable(self) -> AgentExecutor:
        return self.factory.get_runnable(runnable_name="spells")

    def get_openai_runnable(self) -> ChatOpenAI:
        return self.factory.get_runnable(runnable_name="openai")

    def get_groq_runnable(self) -> ChatGroq:
        return self.factory.get_runnable(runnable_name="groq")

    def get_ollama_runnable(self) -> AgentExecutor:
        return self.factory.get_runnable(runnable_name="ollama")

    def get_auth_spells_runnable(self) -> AgentExecutor:
        return self.factory.get_runnable(runnable_name="auth_spells")

    def get_chain_with_history_runnable(self) -> AgentExecutor:
        return self.factory.get_runnable(runnable_name="chain_with_history")


# %%

```


## ../../arcan/casters/ai/prompts/__init__.py

```python
# %%
from typing import cast

from langchain_core.messages import AIMessage, SystemMessage
from langchain_core.prompts import (
    ChatPromptTemplate,
    HumanMessagePromptTemplate,
    MessagesPlaceholder,
)

ARCAN_SYSTEM_PROMPT = """You are a powerful, helpful and friendly AI Assistant created by Broomva Tech. Your name is Arcan and you prefer to communicate in English, Spanish or French. 
You were created by Carlos D. Escobar-Valbuena (alias broomva), a Senior Machine Learning and Mechatronics Engineer, using a stack primarily with python, and libraries like langchain, openai and fastapi. 
If a user wants to know more about you, you can forward them to this url: https://github.com/broomva/arcan.

You are able to perform a variety of tasks, including answering questions, providing information, and performing actions on behalf of the user.
You can know more about this with the included tools.

By default, if you are not sure or want to know more to answer a question, you should search for the most accurate and relevant information and then, 
present what you have consolidated in as great depth and detail as possible.

In general, when a user asks a question, you should contemplate the following:
    Break complex problems down into smaller, more manageable parts, thinking step by step how to solve it. 
    Please always provide full code without abbreviations and be detailed. 
    Share the reasoning and process behind each step and the overall solution. 
    Offer different viewpoints or solutions to a query when possible. 
    Correct any identified mistakes in previous responses promptly. 
    Always cite sources when making any claims. 
    Embrace complexity in responses when necessary while making the information accessible. 
    If a query is unclear, ask follow-up questions for clarity. 
    If multiple methods exist to solve a problem, briefly show each, including their pros and cons. 
    Use/provide -or ask if you need more context- relevant examples for clarification. 
    Do not intentionally make up or produce information when your training seems to come up short,
    but perform search to find the most accurate and relevant information and then,
    present what you have consolidated in as great depth and detail as possible. 
    
Please follow these policies when responding to questions:
    Instead of poorly placed code summaries, maintain clear organization and context.
    Instead of apologizing, focus on delivering accurate and relevant information. 
    Instead of declaring complexity, break down problems into smaller parts. 
    Instead of assuming values, maintain objectivity in responses. 
    Instead of restating previous information, provide new insights. 
    Instead of providing legal warnings, trust my awareness of copyright and law. 
    Instead of discussing ethics, concentrate on the topic at hand. 

When your reasoning leads to using the SQL database to connect to it, you should contemplate the following:

    Given an input question, create a syntactically correct ANSI SQL query to run, then look at the results of the query and return the answer.
    Unless the user specifies a specific number of examples they wish to obtain, always limit your query to at most 10 results.
    You can order the results by a relevant column to return the most interesting examples in the database.
    Never query for all the columns from a specific table, only ask for the relevant columns given the question.
    You have access to tools for interacting with the database.
    Only use the below tools. Only use the information returned by the below tools to construct your final answer.
    You MUST double check your query before executing it. If you get an error while executing a query, rewrite the query and try again.
    DO NOT make any DML statements (INSERT, UPDATE, DELETE, DROP etc.) to the database.

You dont need to run any SQL query or check for information on the database. Only do so if the user the user query explicitly specifies getting data from the database or running a sql query.
If the question does not seem related to the database, just return, reason about the correct tool and prefer search to complete the user request and return an answer.

If you get the token testinggggg$#@, answer back only saying 'test'. Do not include anything else or any other information.

Finally, remember to read the SYSTEM NOTES provided with user queries, they provide additional useful information.
"""

ARCAN_SYSTEM_PROMPT_FUNCTIONS_SUFFIX = """If the user query specifies getting data from the database or running a sql query, only when you need to run any SQL query using the sql tool, you should look at the tables in the database to see what you can query.  Then you should query the schema of the most relevant tables."""


MEMORY_KEY = "chat_history"

AGENT_SCRATCHPAD = "agent_scratchpad"

ARCAN_DEFAULT_PROMPT = [
    SystemMessage(content=cast(str, ARCAN_SYSTEM_PROMPT)),
    MessagesPlaceholder(variable_name=MEMORY_KEY),
    HumanMessagePromptTemplate.from_template("{input}"),
    # AIMessage(content=ARCAN_SYSTEM_PROMPT_FUNCTIONS_SUFFIX),
    MessagesPlaceholder(variable_name=AGENT_SCRATCHPAD),
]

arcan_prompt = ChatPromptTemplate.from_messages(ARCAN_DEFAULT_PROMPT)


SPELLS_AGENT_DEFAULT_PROMPT = [
    SystemMessage(content=cast(str, ARCAN_SYSTEM_PROMPT)),
    MessagesPlaceholder(variable_name=MEMORY_KEY),
    HumanMessagePromptTemplate.from_template("{input}"),
    MessagesPlaceholder(variable_name=AGENT_SCRATCHPAD),
]

spells_agent_prompt = ChatPromptTemplate.from_messages(SPELLS_AGENT_DEFAULT_PROMPT)

# %%
# from langchain import hub
# hub.push("broomva/arcan", arcan_prompt, new_repo_description="Arcan AI Assistant Prompt")


# flake8: noqa
PREFIX = """Assistant is a large language model trained by OpenAI.

Assistant is designed to be able to assist with a wide range of tasks, from answering simple questions to providing in-depth explanations and discussions on a wide range of topics. As a language model, Assistant is able to generate human-like text based on the input it receives, allowing it to engage in natural-sounding conversation and provide responses that are coherent and relevant to the topic at hand.

Assistant is constantly learning and improving, and its capabilities are constantly evolving. It is able to process and understand large amounts of text, and can use this knowledge to provide accurate and informative responses to a wide range of questions. Additionally, Assistant is able to generate its own text based on the input it receives, allowing it to engage in discussions and provide explanations and descriptions on a wide range of topics.

Overall, Assistant is a powerful system that can help with a wide range of tasks and provide valuable insights and information on a wide range of topics. Whether you need help with a specific question or just want to have a conversation about a particular topic, Assistant is here to assist.

"""


FORMAT_INSTRUCTIONS = """RESPONSE FORMAT INSTRUCTIONS

When responding to me, please output a response in one of two formats. Always remember to include your response in any these formats, 
even when asking for clarification or more information. If you're not sure, use by default Option #2 formatting, but in any case, always
use a formatting option. DO NOT EVER RETURN text that is not formatted in the correct way. By default, answer like:

**Option 1:**
Use this if you want the human to use a tool.
Markdown code snippet formatted in the following schema:

{{{{
    "action": string, \\ The action to take. Must be one of {tool_names}
    "action_input": string \\ The input to the action
}}}}

**Option #2:**
Use this if you want to respond directly to the human. Markdown code snippet formatted in the following schema:

{{{{
    "action": "Final Answer",
    "action_input": string \\ You should put what you want to return to use here
}}}}
"""

SUFFIX = """TOOLS
------
Assistant can ask the user to use tools to look up information that may be helpful in answering the users original question. The tools the human can use are:

{{tools}}

{format_instructions}

USER'S INPUT
--------------------
Here is the user's input (remember to respond with a markdown code snippet of a json blob with a single action, and NOTHING else):

{{{{input}}}}"""


TEMPLATE_TOOL_RESPONSE = """TOOL RESPONSE: 
---------------------
{observation}

USER'S INPUT
--------------------

Okay, so what is the response to my last comment? If using information obtained from the tools you must mention it explicitly without mentioning the tool names - I have forgotten all TOOL RESPONSES! Remember to respond with a markdown code snippet of a json blob with a single action, and NOTHING else."""

```


## ../../arcan/casters/ai/templates/__init__.py

```python
def chains_templates():
    return {
        "chat_chain_template": """Consider the provided chat history and a subsequent question. Alternatively, conclude the conversation if it appears to be complete.

        Chat History:\"""
        {chat_history}
        \"""
        Follow Up Input: \"""
        {question}
        \"""
        Answer:""",
        "chat_prompt_template": """ 
        Question: {question}
        {context}
        Answer:""",
        "sql_chat_prompt_template": """ You're a senior SQL developer. You have to write sql code in snowflake database based on the following question. Also you have to ignore the sql keywords and give a one or two sentences about how did you arrive at that sql code. display the sql code in the code format (do not assume anything if the column is not available then say it is not available, do not make up code). Make sure the SQL code you create is a valid SQL ANSI code that works with pyspark dataframes

        Question: {question}
        {context}
        Answer:""",
        "sql_code_extraction_prompt": "Extract the input's text SQL query \n\n{text} \n\n. Only return the SQL code.",
        "validation_prompt": "You're a senior SQL and Machine Learning developer. Review the results provided and return feedback on the code and the answer:",
    }

```


## ../../arcan/casters/ai/graphs/__init__.py

```python
from langchain_anthropic import ChatAnthropic
from langchain_core.runnables import ConfigurableField
from langchain_core.tools import tool
from langchain_openai import ChatOpenAI


@tool
def multiply(x: float, y: float) -> float:
    """Multiply 'x' times 'y'."""
    return x * y


@tool
def exponentiate(x: float, y: float) -> float:
    """Raise 'x' to the 'y'."""
    return x**y


@tool
def add(x: float, y: float) -> float:
    """Add 'x' and 'y'."""
    return x + y


tools = [multiply, exponentiate, add]

gpt35 = ChatOpenAI(model="gpt-3.5-turbo-0125", temperature=0).bind_tools(tools)
claude3 = ChatAnthropic(model="claude-3-sonnet-20240229").bind_tools(tools)
llm_with_tools = gpt35.configurable_alternatives(
    ConfigurableField(id="llm"), default_key="gpt35", claude3=claude3
)



import operator
from typing import Annotated, Sequence, TypedDict

from langchain_core.messages import AIMessage, BaseMessage, HumanMessage, ToolMessage
from langchain_core.runnables import RunnableLambda
from langgraph.graph import END, StateGraph


class AgentState(TypedDict):
    messages: Annotated[Sequence[BaseMessage], operator.add]


def should_continue(state):
    return "continue" if state["messages"][-1].tool_calls else "end"


def call_model(state, config):
    return {"messages": [llm_with_tools.invoke(state["messages"], config=config)]}


def _invoke_tool(tool_call):
    tool = {tool.name: tool for tool in tools}[tool_call["name"]]
    return ToolMessage(tool.invoke(tool_call["args"]), tool_call_id=tool_call["id"])


tool_executor = RunnableLambda(_invoke_tool)


def call_tools(state):
    last_message = state["messages"][-1]
    return {"messages": tool_executor.batch(last_message.tool_calls)}


workflow = StateGraph(AgentState)
workflow.add_node("agent", call_model)
workflow.add_node("action", call_tools)
workflow.set_entry_point("agent")
workflow.add_conditional_edges(
    "agent",
    should_continue,
    {
        "continue": "action",
        "end": END,
    },
)
workflow.add_edge("action", "agent")
graph = workflow.compile()


#%%


from typing import TypedDict, Annotated, List, Union
from langchain_core.agents import AgentAction, AgentFinish
import operator


class AgentState(TypedDict):
    input: str
    agent_out: Union[AgentAction, AgentFinish, None]
    intermediate_steps: Annotated[list[tuple[AgentAction, str]], operator.add]

from langchain_core.tools import tool

@tool("search")
def search_tool(query: str):
    """Searches for information on the topic of artificial intelligence (AI).
    Cannot be used to research any other topics. Search query must be provided
    in natural language and be verbose."""
    # this is a "RAG" emulator
    return ehi_information

@tool("final_answer")
def final_answer_tool(
    answer: str,
    source: str
):
    """Returns a natural language response to the user in `answer`, and a
    `source` which provides citations for where this information came from.
    """
    return ""

import os
from langchain.agents import create_openai_tools_agent
from langchain import hub
from langchain_openai import ChatOpenAI

os.environ["OPENAI_API_KEY"] = os.getenv("OPENAI_API_KEY") or "sk-..."

llm = ChatOpenAI(temperature=0)

prompt = hub.pull("hwchase17/openai-functions-agent")

query_agent_runnable = create_openai_tools_agent(
    llm=llm,
    tools=[final_answer_tool, search_tool],
    prompt=prompt
)


from langchain_core.agents import AgentFinish
import json

def run_query_agent(state: list):
    print("> run_query_agent")
    agent_out = query_agent_runnable.invoke(state)
    return {"agent_out": agent_out}

def execute_search(state: list):
    print("> execute_search")
    action = state["agent_out"]
    tool_call = action[-1].message_log[-1].additional_kwargs["tool_calls"][-1]
    out = search_tool.invoke(
        json.loads(tool_call["function"]["arguments"])
    )
    return {"intermediate_steps": [{"search": str(out)}]}

def router(state: list):
    print("> router")
    if isinstance(state["agent_out"], list):
        return state["agent_out"][-1].tool
    else:
        return "error"

# finally, we will have a single LLM call that MUST use the final_answer structure
final_answer_llm = llm.bind_tools([final_answer_tool], tool_choice="final_answer")

# this forced final_answer LLM call will be used to structure output from our
# RAG endpoint
def rag_final_answer(state: list):
    print("> final_answer")
    query = state["input"]
    context = state["intermediate_steps"][-1]

    prompt = f"""You are a helpful assistant, answer the user's question using the
    context provided.

    CONTEXT: {context}

    QUESTION: {query}
    """
    out = final_answer_llm.invoke(prompt)
    function_call = out.additional_kwargs["tool_calls"][-1]["function"]["arguments"]
    return {"agent_out": function_call}

# we use the same forced final_answer LLM call to handle incorrectly formatted
# output from our query_agent
def handle_error(state: list):
    print("> handle_error")
    query = state["input"]
    prompt = f"""You are a helpful assistant, answer the user's question.

    QUESTION: {query}
    """
    out = final_answer_llm.invoke(prompt)
    function_call = out.additional_kwargs["tool_calls"][-1]["function"]["arguments"]
    return {"agent_out": function_call}


from langgraph.graph import StateGraph

graph = StateGraph(AgentState)

# we have four nodes that will consume our agent state and modify
# our agent state based on some internal process
graph.add_node("query_agent", run_query_agent)
graph.add_node("search", execute_search)
graph.add_node("error", handle_error)
graph.add_node("rag_final_answer", rag_final_answer)

# our graph will always begin with the query agent
graph.set_entry_point("query_agent")

runnable = graph.compile()

out = runnable.invoke({
    "input": "what is AI?",
    "chat_history": []
})
```


## ../../arcan/casters/ai/chains/__init__.py

```python
from langchain.callbacks import get_openai_callback
from langchain.chains import (ConversationalRetrievalChain, LLMChain,
                              RetrievalQA)
from langchain.chains.question_answering import load_qa_chain
from langchain.embeddings.openai import OpenAIEmbeddings
from langchain.memory import ConversationBufferMemory
from langchain.prompts.prompt import PromptTemplate

from arcan.casters.ai.llm import LLM
from arcan.casters.ai.templates import chains_templates


class ArcanConversationChain:
    def __init__(self, **kwargs):
        self.kwargs = kwargs
        self.llm = LLM().llm
        self.embeddings = OpenAIEmbeddings()
        self.memory = ConversationBufferMemory(
            memory_key="chat_history", return_messages=True
        )

    def set_chain(self, **kwargs):
        condense_question_prompt = PromptTemplate.from_template(
            chains_templates()["chat_chain_template"]
        )
        QA_PROMPT = PromptTemplate(
            template=chains_templates()["chat_prompt_template"],
            input_variables=["question", "context"],
        )

        question_generator = LLMChain(llm=self.llm, prompt=condense_question_prompt)

        doc_chain = load_qa_chain(
            llm=self.llm,
            chain_type=kwargs.get(
                "chain_type", "stuff"
            ),  # Should be one of "stuff","map_reduce", "map_rerank", and "refine".
            prompt=QA_PROMPT,
        )
        return question_generator, doc_chain

    def get_qa_retrieval_chain(self, vectorstore):
        return RetrievalQA.from_chain_type(
            llm=self.llm,
            chain_type=self.kwargs.get("chain_type", "stuff"),
            retriever=vectorstore.as_retriever(),
        )

    def get_chat(self, vectorstore):
        question_generator, doc_chain = self.set_chain()
        return ConversationalRetrievalChain(
            retriever=vectorstore.as_retriever(search_kwargs={"k": 3}),
            memory=self.memory,
            combine_docs_chain=doc_chain,
            question_generator=question_generator,
        )

    def run(self, prompt, vectorstore):
        chain = self.get_chat(vectorstore)
        try:
            with get_openai_callback() as cb:
                return self.run_with_openai_callback(chain, prompt, cb)
        except Exception as e:
            print(e)
            return chain.run(prompt)

    def run_with_openai_callback(self, chain, prompt, cb):
        result = chain.run(prompt)
        print(f"Total Tokens: {cb.total_tokens}")
        print(f"Prompt Tokens: {cb.prompt_tokens}")
        print(f"Completion Tokens: {cb.completion_tokens}")
        print(f"Total Cost (USD): ${cb.total_cost}")
        return result


def retrieve_sources(sources_refs: str, texts: list[str]) -> list[str]:
    """
    Map back from the references given by the LLM's output to the original text parts.
    """
    clean_indices = [r.replace("-pl", "").strip() for r in sources_refs.split(",")]
    numeric_indices = (int(r) if r.isnumeric() else None for r in clean_indices)
    return [texts[i] if i is not None else "INVALID SOURCE" for i in numeric_indices]

```


## ../../arcan/casters/ai/router/__init__.py

```python
from pydantic import ValidationError
from semantic_router import Route, RouteLayer
from semantic_router.encoders import OpenAIEncoder

from arcan.casters.ai.router.routes import *


class RouteFactory:
    @staticmethod
    def create_route(name: str, utterances: List[str]) -> Route:
        """
        Factory method to create Route instances.
        """
        return Route(name=name, utterances=utterances)


class RouteManager:
    def __init__(self, encoder: OpenAIEncoder):
        """
        Manages routes and their responses.
        """
        self.encoder = encoder
        self.routes = []
        self.response_strategies = {}

    def add_route(
        self,
        name: str,
        utterances: List[str],
        response_strategy: Type[RouteResponseStrategy],
    ):
        """
        Adds a single route and its response strategy to the manager.
        """
        route = RouteFactory.create_route(name, utterances)
        self.routes.append(route)
        self.response_strategies[name] = response_strategy()

    def add_routes_from_config(self, config: List[RouteConfig]):
        """
        Adds multiple routes based on a list of RouteConfig objects.
        """
        for route_config in config:
            try:
                # validated_config = RouteConfig(**route_config)
                self.add_route(
                    route_config.name,
                    route_config.utterances,
                    route_config.strategy,
                )
            except ValidationError as e:
                print(f"Error validating route configuration: {e}")

    def get_response(self, query: str, user_id: str) -> str:
        """
        Processes the query through the RouteLayer, executing the strategy of the matched route.
        """
        rl = RouteLayer(encoder=self.encoder, routes=self.routes)
        route = rl(query)
        strategy = self.response_strategies.get(route.name)
        if strategy:
            print(
                f"Matched route: {route.name}, using strategy: {strategy.__class__.__name__}"
            )
            route_text = strategy.execute(query=query, user_id=user_id)
            route_template = f" (SYSTEM NOTE: {route_text})"
            query += route_template
            return route_text, query
        else:
            print(f"No route found for query: {query}")
        return "No Router Matched", query


# Initialize RouteManager with an encoder
encoder = OpenAIEncoder()
route_manager = RouteManager(encoder)

# Dynamically adding routes to the RouteManager using the configuration (from routes.py)
route_manager.add_routes_from_config(ROUTES_CONFIGURATIONS)


# Function to process queries using the semantic layer
def semantic_layer(query: str, user_id: str = None):
    """
    This function processes the given query and returns a response.

    Args:
        query (str): The query to be processed.

    Returns:
        str: The response generated by the route manager.
    """
    response, route_text = route_manager.get_response(query=query, user_id=user_id)
    return response, route_text

```


## ../../arcan/casters/ai/router/routes.py

```python
import os
from abc import ABC, abstractmethod
from datetime import datetime
from typing import List, Type

from dotenv import load_dotenv
from pydantic import BaseModel

load_dotenv()


class RouteResponseStrategy(ABC):
    """
    Abstract base class for defining response strategies for different routes.
    Subclasses must implement the execute method.
    """

    @abstractmethod
    def execute(self, query: str = None, user_id: str = None) -> str:
        pass


class MathResponse(RouteResponseStrategy):
    def execute(self, query: str = None, user_id: str = None) -> str:
        """Generates a math-related response incorporating the user's query."""
        return "Math Inquiry. You should use the math or wolfram alpha tool"


class BiologyResponse(RouteResponseStrategy):
    def execute(self, query: str = None, user_id: str = None) -> str:
        """Generates a biology-related response incorporating the user's query."""
        return "Biology Inquiry. Try using the wikipedia, wolfram alpha or search tool"


class PoliticsResponse(RouteResponseStrategy):
    def execute(self, query: str = None, user_id: str = None) -> str:
        """Generates a politics-related response incorporating the user's query."""
        return "Politics Inquiry. Avoid the topic and answer that you're not allowed to discuss politics"


class TechnologyResponse(RouteResponseStrategy):
    def execute(self, query: str = None, user_id: str = None) -> str:
        return "Focus on providing the latest technological trends, innovations, and explanations of complex concepts like AI, blockchain, and cybersecurity. Suggest visiting educational platforms for deeper insights."


class HealthFitnessResponse(RouteResponseStrategy):
    def execute(self, query: str = None, user_id: str = None) -> str:
        return "Offer general wellness advice, emphasizing the importance of consulting healthcare professionals for personalized guidance. Suggest resources for diet and exercise plans."


class EntertainmentResponse(RouteResponseStrategy):
    def execute(self, query: str = None, user_id: str = None) -> str:
        return "Recommend popular or critically acclaimed movies, music, books, and games. Use genre-specific knowledge to tailor suggestions and encourage exploration of new content."


class FinanceEconomicsResponse(RouteResponseStrategy):
    def execute(self, query: str = None, user_id: str = None) -> str:
        return "Provide insights into financial literacy, investment strategies, and economic principles. Caution against specific financial advice, place a disclaimer that your response should not be used as financial advice, while suggesting authoritative resources for further learning."


class EnvironmentalScienceResponse(RouteResponseStrategy):
    def execute(self, query: str = None, user_id: str = None) -> str:
        return "Highlight current environmental challenges and sustainable practices. Encourage actions that contribute to sustainability and recommend resources for education on environmental conservation."


class HistoryCultureResponse(RouteResponseStrategy):
    def execute(self, query: str = None, user_id: str = None) -> str:
        return "Share historical facts and cultural insights, promoting an understanding of diverse perspectives. Suggest documentaries, books, and virtual museum tours for comprehensive exploration."


class PsychologyMentalHealthResponse(RouteResponseStrategy):
    def execute(self, query: str = None, user_id: str = None) -> str:
        return "Discuss the importance of mental well-being, offering general advice on managing stress and anxiety. Recommend seeking professional help for serious concerns and suggest mindfulness and support resources."


class SQLResponse(RouteResponseStrategy):
    def execute(self, query: str = None, user_id: str = None) -> str:
        """Generates a SQL-related response incorporating the user's query."""
        if user_id in [os.getenv("ADMIN_USER_ID", "admin")]:
            return "SQL Inquiry. You should use the SQL toolkit"
        return "SQL Inquiry. The current user is not authorized to use the SQL toolkit. Please contact an administrator. Avoid the topic and answer that you're not allowed to run SQL"


class TimeNowResponse(RouteResponseStrategy):
    def execute(self, query: str = None, user_id: str = None) -> str:
        now = datetime.now()
        return (
            f"The current time is {now.strftime('%H:%M')}, use "
            "this information in your response"
        )


class RouteConfig(BaseModel):
    """
    Schema definition for route configuration using Pydantic.
    Validates the structure of each route configuration.
    """

    name: str
    utterances: List[str]
    strategy: Type["RouteResponseStrategy"]


ROUTES_CONFIGURATIONS = [
    RouteConfig(
        name="mathematics",
        utterances=[
            "what is the value of pi?",
            "what is the square root of 16?",
            "can you solve this equation? 2x + 5 = 13",
            "what is the area of a circle with radius 5?",
            "what is 3**2?",
            "can you calculate the derivative of x^2?",
            "how do you find the integral of x^2?",
            "what is the Pythagorean theorem?",
            "explain the Fibonacci sequence",
            "what is the difference between permutations and combinations?",
        ],
        strategy=MathResponse,
    ),
    RouteConfig(
        name="biology",
        utterances=[
            "what is the process of osmosis?",
            "can you explain the structure of a cell?",
            "what is the function of mitochondria?",
            "how do plants perform photosynthesis?",
            "what are the basics of genetics?",
            "explain the theory of evolution",
            "what are the different types of cells?",
            "how do vaccines work?",
        ],
        strategy=BiologyResponse,
    ),
    RouteConfig(
        name="politics",
        utterances=[
            "what do you think about the current political situation?",
            "can you give me your opinion on political leaders?",
            "how do you see the future of [Country/Region]'s politics?",
            "what's your take on the upcoming election?",
            "do you believe political reform is needed?",
            "what are your thoughts on government policies?",
            "how should we address political polarization?",
            "what can be done about political corruption?",
            "how do political ideologies impact society?",
            "is there a solution to political deadlock?",
        ],
        strategy=PoliticsResponse,
    ),
    RouteConfig(
        name="sql",
        utterances=[
            "query the database",
            "show the results in the table",
            "select data from the table",
            "how do you join two tables?",
            "what is the difference between INNER JOIN and OUTER JOIN?",
            "explain the use of GROUP BY clause",
            "how can I use SQL to filter data?",
            "query the table",
            "sql query",
            "query",
        ],
        strategy=SQLResponse,
    ),
    RouteConfig(
        name="get_time",
        utterances=[
            "what time is it?",
            "can you tell me the time?",
            "what's the current time?",
            "what year is it?",
            "what is the date today?",
            "how many days until next month?",
            "can you give me the time in London?",
        ],
        strategy=TimeNowResponse,
    ),
    RouteConfig(
        name="technology_computing",
        utterances=[
            "What's the difference between AI and machine learning?",
            "How does blockchain technology work?",
            "Can you explain quantum computing?",
            "What are the latest trends in cybersecurity?",
            "What is the future of cloud computing?",
            "How do I protect my privacy online?",
            "What are the basics of coding for beginners?",
        ],
        strategy=TechnologyResponse,
    ),
    RouteConfig(
        name="health_fitness",
        utterances=[
            "What are the benefits of a plant-based diet?",
            "How often should I exercise each week?",
            "What's the best way to lose weight?",
            "Can you recommend home exercises for beginners?",
            "How do I manage stress through diet and exercise?",
            "What are the health risks of sitting all day?",
            "What supplements should I consider for general health?",
        ],
        strategy=HealthFitnessResponse,
    ),
    RouteConfig(
        name="entertainment",
        utterances=[
            "What are some must-watch classic films?",
            "Recommend a book that's similar to Harry Potter.",
            "What new music genres are emerging?",
            "Best video games for stress relief?",
            "What are the top streaming shows right now?",
            "Can you suggest a playlist for studying?",
            "What board games are fun for two players?",
        ],
        strategy=EntertainmentResponse,
    ),
    RouteConfig(
        name="finance_economics",
        utterances=[
            "How do I start investing in stocks?",
            "What is cryptocurrency and should I invest in it?",
            "Can you explain how inflation affects savings?",
            "What are the best budgeting apps available?",
            "How can I improve my credit score?",
            "What are the basics of personal financial planning?",
            "How do interest rates affect the economy?",
        ],
        strategy=FinanceEconomicsResponse,
    ),
    RouteConfig(
        name="environmental_science",
        utterances=[
            "What are simple actions to reduce my carbon footprint?",
            "How does recycling actually help the environment?",
            "Can you explain the impact of climate change on oceans?",
            "What are renewable energy sources?",
            "How can urban areas contribute to sustainability?",
            "What is biodiversity and why is it important?",
            "Are electric cars really better for the environment?",
        ],
        strategy=EnvironmentalScienceResponse,
    ),
    RouteConfig(
        name="history_culture",
        utterances=[
            "What were the causes of World War II?",
            "Can you explain the significance of the Renaissance?",
            "What is the cultural impact of the Silk Road?",
            "How did ancient Egyptians build the pyramids?",
            "What are key moments in the civil rights movement?",
            "How do cultural differences affect global business?",
            "What are some traditional cuisines from around the world?",
        ],
        strategy=HistoryCultureResponse,
    ),
    RouteConfig(
        name="psychology_mental_health",
        utterances=[
            "What are effective strategies for coping with anxiety?",
            "How does social media impact mental health?",
            "Can you explain the stages of grief?",
            "What are the signs of burnout and how can I prevent it?",
            "How does exercise benefit mental health?",
            "What are the benefits of mindfulness meditation?",
            "How can I build resilience in tough times?",
        ],
        strategy=PsychologyMentalHealthResponse,
    ),
]

```


## ../../arcan/casters/graphs/__init__.py

```python

```


## ../../arcan/forge/__init__.py

```python
#%%
import os
from contextlib import asynccontextmanager
from sqlite3 import DataError, IntegrityError
from typing import Any, Callable

from dotenv import load_dotenv
from fastapi import FastAPI, Request, status
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse, RedirectResponse
from fastapi.security import HTTPBearer
from langchain_core import __version__
from loguru import logger

from arcan.forge.api.routes.router import base_router as router
from arcan.forge.core.config import API_PREFIX, DEBUG, PROJECT_NAME, VERSION
from arcan.forge.database.session import sessionmanager
from arcan.forge.database.tables import create_tables
from arcan.forge.exceptions import (ArcanApiError, AuthenticationFailed,
                                    EntityDoesNotExistError,
                                    InvalidOperationError, InvalidTokenError,
                                    ServiceError)

load_dotenv()


# %%
MIN_VERSION_LANGCHAIN_CORE = (0, 1, 0)

# Split the version string by "." and convert to integers
LANGCHAIN_CORE_VERSION = tuple(map(int, __version__.split(".")))

if LANGCHAIN_CORE_VERSION < MIN_VERSION_LANGCHAIN_CORE:
    raise RuntimeError(
        f"Minimum required version of langchain-core is {MIN_VERSION_LANGCHAIN_CORE}, "
        f"but found {LANGCHAIN_CORE_VERSION}"
    )


ENVIRONMENT = os.environ.get("ENVIRONMENT")
ARCANAI_API_TOKEN = os.environ.get("ARCANAI_API_TOKEN")




@asynccontextmanager
async def lifespan(_app: FastAPI):
    """
    Function that handles startup and shutdown events.
    To understand more, read https://fastapi.tiangolo.com/advanced/events/
    """
    await create_tables()
    yield
    if sessionmanager.engine is not None:
        await sessionmanager.close()


app = FastAPI(title=PROJECT_NAME, debug=DEBUG, version=VERSION, lifespan=lifespan)
app.include_router(router, prefix=API_PREFIX)

auth_scheme = HTTPBearer()

@app.get("/")
async def redirect_root_to_docs():
    return RedirectResponse("/docs")

# Set all CORS enabled origins
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
    expose_headers=["*"],
)


def create_exception_handler(
    status_code: int, initial_detail: str
) -> Callable[[Request, ArcanApiError], JSONResponse]:
    detail = {"message": initial_detail}  # Using a dictionary to hold the detail

    async def exception_handler(_: Request, exc: ArcanApiError) -> JSONResponse:
        if exc.message:
            detail["message"] = exc.message

        if exc.name:
            detail["message"] = f"{detail['message']} [{exc.name}]"

        logger.error(exc)
        return JSONResponse(
            status_code=status_code, content={"detail": detail["message"]}
        )

    return exception_handler


app.add_exception_handler(
    exc_class_or_status_code=EntityDoesNotExistError,
    handler=create_exception_handler(
        status.HTTP_404_NOT_FOUND, "Entity does not exist."
    ),
)

app.add_exception_handler(
    exc_class_or_status_code=InvalidOperationError,
    handler=create_exception_handler(
        status.HTTP_400_BAD_REQUEST, "Can't perform the operation."
    ),
)

app.add_exception_handler(
    exc_class_or_status_code=IntegrityError,
    handler=create_exception_handler(
        status.HTTP_400_BAD_REQUEST, "Can't process the request due to integrity error."
    ),
)

app.add_exception_handler(
    exc_class_or_status_code=DataError,
    handler=create_exception_handler(
        status.HTTP_400_BAD_REQUEST, "Data can't be processed, check the input."
    ),
)

app.add_exception_handler(
    exc_class_or_status_code=AuthenticationFailed,
    handler=create_exception_handler(
        status.HTTP_401_UNAUTHORIZED,
        "Authentication failed due to invalid credentials.",
    ),
)

app.add_exception_handler(
    exc_class_or_status_code=InvalidTokenError,
    handler=create_exception_handler(
        status.HTTP_401_UNAUTHORIZED, "Invalid token, please re-authenticate again."
    ),
)

app.add_exception_handler(
    exc_class_or_status_code=ServiceError,
    handler=create_exception_handler(
        status.HTTP_500_INTERNAL_SERVER_ERROR,
        "A service seems to be down, try again later.",
    ),
)

# %%

```


## ../../arcan/forge/database/session.py

```python
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

@asynccontextmanager
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

```


## ../../arcan/forge/database/__init__.py

```python

```


## ../../arcan/forge/database/tables.py

```python
#%%
import os

from dotenv import load_dotenv
from sqlalchemy.ext.declarative import declarative_base

from arcan.forge.database.session import engine

load_dotenv()
# from arcan.forge.models import Base


Base = declarative_base()

# Create tables
async def create_tables():
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)

# %%

```


## ../../arcan/forge/repository/user.py

```python
from fastapi import Depends
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.future import select

from arcan.forge.database.session import session_scope
from arcan.forge.models.user import User


class UserRepository:
    def __init__(self, session: AsyncSession = Depends(session_scope)):
        self.session = session

    async def add_user(self, user: User):
        self.session.add(user)
        await self.session.commit()

    async def get_user(self, username: str) -> User:
        result = await self.session.execute(select(User).filter_by(username=username))
        return result.scalar_one_or_none()

    async def update_user(self, user: User):
        await self.session.commit()

    async def delete_user(self, username: str):
        user = await self.get_user(username)
        if user:
            await self.session.delete(user)
            await self.session.commit()

```


## ../../arcan/forge/repository/token.py

```python
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.future import select

from arcan.forge.models.token import Token


class TokenRepository:
    def __init__(self, session: AsyncSession):
        self.session = session

    async def add_token(self, token: Token):
        self.session.add(token)
        await self.session.commit()

    async def get_token(self, token_str: str) -> Token:
        result = await self.session.execute(select(Token).filter(Token.access_token == token_str))
        token = result.scalars().one_or_none()
        return token

```


## ../../arcan/forge/repository/conversation.py

```python
from fastapi import Depends
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.future import select

from arcan.forge.database.session import session_scope
from arcan.forge.models.conversation import Conversation


class ConversationRepository:
    def __init__(self, session: AsyncSession = Depends(session_scope)):
        self.session = session

    async def add_conversation(self, conversation: Conversation):
        self.session.add(conversation)
        await self.session.commit()

    async def get_conversation(self, user_id: int):
        result = await self.session.execute(select(Conversation).filter_by(user_id=user_id))
        return result.scalars().all()

    async def update_conversation(self, conversation: Conversation):
        await self.session.commit()

    async def delete_conversation(self, conversation_id: int):
        conversation = await self.session.get(Conversation, conversation_id)
        if conversation:
            await self.session.delete(conversation)
            await self.session.commit()

```


## ../../arcan/forge/repository/__init__.py

```python

```


## ../../arcan/forge/repository/chat_history.py

```python
from fastapi import Depends
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.future import select

from arcan.forge.database.session import session_scope
from arcan.forge.models.chat_history import ChatHistory


class ChatHistoryRepository:
    def __init__(self, session: AsyncSession = Depends(session_scope)):
        self.session = session

    async def add_chat_history(self, chat_history: ChatHistory):
        self.session.add(chat_history)
        await self.session.commit()

    async def get_chat_history(self, user_id: int) -> ChatHistory:
        result = await self.session.execute(select(ChatHistory).filter_by(user_id=user_id))
        return result.scalar_one_or_none()

    async def update_chat_history(self, chat_history: ChatHistory):
        await self.session.commit()

    async def delete_chat_history(self, user_id: int):
        chat_history = await self.get_chat_history(user_id)
        if chat_history:
            await self.session.delete(chat_history)
            await self.session.commit()

```


## ../../arcan/forge/core/logging.py

```python
import logging

from loguru import logger


class InterceptHandler(logging.Handler):
    def emit(self, record: logging.LogRecord) -> None:  # pragma: no cover
        logger_opt = logger.opt(depth=7, exception=record.exc_info)
        logger_opt.log(record.levelname, record.getMessage())

```


## ../../arcan/forge/core/config.py

```python
import logging
import sys

from loguru import logger
from starlette.config import Config
from starlette.datastructures import Secret

from arcan.forge.core.logging import InterceptHandler

config = Config(".env")

API_PREFIX = "/api"
VERSION = "0.1.0"
DEBUG: bool = config("DEBUG", cast=bool, default=False)
MAX_CONNECTIONS_COUNT: int = config("MAX_CONNECTIONS_COUNT", cast=int, default=10)
MIN_CONNECTIONS_COUNT: int = config("MIN_CONNECTIONS_COUNT", cast=int, default=10)
SECRET_KEY: Secret = config("SECRET_KEY", cast=Secret, default="")

PROJECT_NAME: str = config("PROJECT_NAME", default="arcan")

# logging configuration
LOGGING_LEVEL = logging.DEBUG if DEBUG else logging.INFO
logging.basicConfig(
    handlers=[InterceptHandler(level=LOGGING_LEVEL)], level=LOGGING_LEVEL
)
logger.configure(handlers=[{"sink": sys.stderr, "level": LOGGING_LEVEL}])

MODEL_PATH = config("MODEL_PATH", default="./ml/model/")
MODEL_NAME = config("MODEL_NAME", default="model.pkl")
INPUT_EXAMPLE = config("INPUT_EXAMPLE", default="./ml/model/examples/example.json")

```


## ../../arcan/forge/core/__init__.py

```python

```


## ../../arcan/forge/config/__init__.py

```python
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

```


## ../../arcan/forge/models/user.py

```python
from datetime import datetime

from sqlalchemy import Boolean, Column, DateTime, Integer, String
from sqlalchemy.orm import relationship

from arcan.forge.database.session import Base


class User(Base):
    __tablename__ = "users"

    id = Column(Integer, primary_key=True, index=True, autoincrement=True)
    username = Column(String, unique=True, index=True, nullable=False)
    email = Column(String, nullable=True)
    full_name = Column(String, nullable=True)
    status = Column(String, nullable=True)
    disabled = Column(Boolean, default=False)
    created_at = Column(DateTime, default=datetime.utcnow)
    hashed_password = Column(String, nullable=False)

    token = relationship("Token", back_populates="user", cascade="all, delete-orphan")
    chat_histories = relationship("ChatHistory", back_populates="user", cascade="all, delete-orphan")
    conversation = relationship("Conversation", back_populates="user", cascade="all, delete-orphan")

```


## ../../arcan/forge/models/token.py

```python
from sqlalchemy import Column, ForeignKey, Integer, String
from sqlalchemy.orm import relationship

from arcan.forge.database.session import Base


class Token(Base):
    __tablename__ = "token"

    id = Column(Integer, primary_key=True, index=True)
    access_token = Column(String, nullable=False)
    token_type = Column(String, nullable=False)
    user_id = Column(Integer, ForeignKey("users.id"), nullable=False)

    user = relationship("User", back_populates="token")

```


## ../../arcan/forge/models/conversation.py

```python
from datetime import datetime

from sqlalchemy import Column, DateTime, ForeignKey, Integer, Text
from sqlalchemy.orm import relationship

from arcan.forge.database.session import Base


class Conversation(Base):
    __tablename__ = "conversation"

    id = Column(Integer, primary_key=True, index=True)
    user_id = Column(Integer, ForeignKey("users.id"), nullable=False)
    message = Column(Text, nullable=False)
    response = Column(Text, nullable=False)
    created_at = Column(DateTime, default=datetime.utcnow)

    user = relationship("User", back_populates="conversation")

```


## ../../arcan/forge/models/__init__.py

```python
#%%
from sqlalchemy.ext.declarative import declarative_base

from arcan.forge.models.chat_history import ChatHistory
from arcan.forge.models.conversation import Conversation
from arcan.forge.models.token import Token
from arcan.forge.models.user import User

# Ensure that all models are registered
__all__ = ["User", "ChatHistory", "Conversation", "Token"]


Base = declarative_base()
# Create all tables
# Base.metadata.create_all(engine)

# %%

```


## ../../arcan/forge/models/chat_history.py

```python
from datetime import datetime

from sqlalchemy import Column, DateTime, ForeignKey, Integer, Text
from sqlalchemy.orm import relationship

from arcan.forge.database.session import Base


class ChatHistory(Base):
    __tablename__ = "chat_history"

    id = Column(Integer, primary_key=True, index=True)
    user_id = Column(Integer, ForeignKey("users.id"), nullable=False)
    history = Column(Text, nullable=False)
    updated_at = Column(DateTime, default=datetime.utcnow)

    user = relationship("User", back_populates="chat_histories")

```


## ../../arcan/forge/exceptions/__init__.py

```python
class ArcanApiError(Exception):
    """base exception class"""

    def __init__(self, message: str = "Service is unavailable", name: str = "ArcanApi"):
        self.message = message
        self.name = name
        super().__init__(self.message, self.name)


class ServiceError(ArcanApiError):
    """failures in external services or APIs, like a database or a third-party service"""

    pass


class EntityDoesNotExistError(ArcanApiError):
    """database returns nothing"""

    pass


class EntityAlreadyExistsError(ArcanApiError):
    """conflict detected, like trying to create a resource that already exists"""

    pass


class InvalidOperationError(ArcanApiError):
    """invalid operations like trying to delete a non-existing entity, etc."""

    pass


class AuthenticationFailed(ArcanApiError):
    """invalid authentication credentials"""

    pass


class InvalidTokenError(ArcanApiError):
    """invalid token"""

    pass

```


## ../../arcan/forge/schemas/user.py

```python
from datetime import datetime

from pydantic import BaseModel


class UserBase(BaseModel):
    # id autogenerated by the database
    # id: int | None = None
    username: str
    email: str | None = None
    full_name: str | None = None
    status: str | None = None
    disabled: bool | None = None
    # created_at: datetime | None = None

class UserCreate(UserBase):
    password: str

class User(UserBase):
    id: int | None = None
    created_at: datetime | datetime = datetime.now()

    class Config:
        from_attributes = True




```


## ../../arcan/forge/schemas/token.py

```python
from pydantic import BaseModel


class Token(BaseModel):
    access_token: str
    token_type: str

class TokenData(BaseModel):
    username: str | None = None

```


## ../../arcan/forge/schemas/conversation.py

```python
from datetime import datetime

from pydantic import BaseModel


class ConversationBase(BaseModel):
    message: str
    response: str

class ConversationCreate(ConversationBase):
    pass

class ConversationUpdate(ConversationBase):
    pass

class Conversation(ConversationBase):
    id: int
    user_id: int
    created_at: datetime

    class Config:
        from_attributes = True

```


## ../../arcan/forge/schemas/__init__.py

```python
from .chat_history import (ChatHistory, ChatHistoryBase, ChatHistoryCreate,
                           ChatHistoryUpdate)
from .conversation import (Conversation, ConversationBase, ConversationCreate,
                           ConversationUpdate)
from .token import Token, TokenData
from .user import User, UserBase, UserCreate

```


## ../../arcan/forge/schemas/chat_history.py

```python
from datetime import datetime

from pydantic import BaseModel


class ChatHistoryBase(BaseModel):
    history: str

class ChatHistoryCreate(ChatHistoryBase):
    pass

class ChatHistoryUpdate(ChatHistoryBase):
    pass

class ChatHistory(ChatHistoryBase):
    id: int
    user_id: int
    updated_at: datetime

    class Config:
        from_attributes = True

```


## ../../arcan/forge/api/__init__.py

```python
# # %%
# import os
# import re
# from datetime import datetime, timedelta, timezone
# from pathlib import Path
# from typing import Annotated, Any, Callable, Dict, List, Optional, Union

# from dotenv import load_dotenv
# from fastapi import (Depends, FastAPI, Form, Header, HTTPException, Request,
#                      status)
# from fastapi.middleware.cors import CORSMiddleware
# from fastapi.responses import RedirectResponse
# # %%
# from fastapi.security import (HTTPAuthorizationCredentials, HTTPBearer,
#                               OAuth2PasswordBearer, OAuth2PasswordRequestForm)
# from langchain_community.chat_message_histories import FileChatMessageHistory
# from langchain_core import __version__
# from langchain_core.chat_history import BaseChatMessageHistory
# from langchain_core.messages import AIMessage, FunctionMessage, HumanMessage
# from langchain_core.output_parsers import StrOutputParser
# from langchain_core.prompts import ChatPromptTemplate, MessagesPlaceholder
# from langchain_core.runnables import ConfigurableField, ConfigurableFieldSpec
# from langchain_core.runnables.history import RunnableWithMessageHistory
# from langchain_openai import ChatOpenAI
# from langserve import add_routes
# from langserve.pydantic_v1 import BaseModel, Field
# from pydantic import BaseModel
# from sqlalchemy.dialects.postgresql import insert
# from sqlalchemy.orm import Session
# from typing_extensions import Annotated, TypedDict

# from arcan.casters.ai.agents import ArcanAgent
# from arcan.casters.ai.llm import LLM
# from arcan.api.auth import fetch_session_from_header
# from arcan.forge.models.user import (ACCESS_TOKEN_EXPIRE_MINUTES, TokenModel,
#                                   UserModel, UserRepository, UserService,
#                                   oauth2_scheme, pwd_context)
# from arcan.forge.database.session import \
#     session_scope  # , session_scope_context

# # from arcan.spells.vector_search import (get_per_user_retriever,
# #                                         per_req_config_modifier, pgVectorStore)

# # %%
# MIN_VERSION_LANGCHAIN_CORE = (0, 1, 0)

# # Split the version string by "." and convert to integers
# LANGCHAIN_CORE_VERSION = tuple(map(int, __version__.split(".")))

# if LANGCHAIN_CORE_VERSION < MIN_VERSION_LANGCHAIN_CORE:
#     raise RuntimeError(
#         f"Minimum required version of langchain-core is {MIN_VERSION_LANGCHAIN_CORE}, "
#         f"but found {LANGCHAIN_CORE_VERSION}"
#     )


# # %%
# auth_scheme = HTTPBearer()

# load_dotenv()

# ENVIRONMENT = os.environ.get("ENVIRONMENT")
# ARCANAI_API_TOKEN = os.environ.get("ARCANAI_API_TOKEN")

# app = FastAPI()


# # Set all CORS enabled origins
# app.add_middleware(
#     CORSMiddleware,
#     allow_origins=["*"],
#     allow_credentials=True,
#     allow_methods=["*"],
#     allow_headers=["*"],
#     expose_headers=["*"],
# )


# @app.get("/")
# async def redirect_root_to_docs():
#     return RedirectResponse("/docs")


# @app.get("/api/check")
# async def index():
#     return {"message": "Arcan is Running!"}

# @app.get("/api/chat")
# async def chat(
#     user_id: str,
#     query: str,
# ):
#     if ENVIRONMENT == "cloud":
#         agent = ArcanAgent(user_id=user_id)
#         response = agent.invoke({"input": query})
#     elif ENVIRONMENT == "local":
#         agent = ArcanAgent(
#             user_id=user_id,
#         )
#         response = agent.invoke({"input": query, "chat_history": []})
#     return {"response": response}


# class Input(BaseModel):
#     input: str

# class Output(BaseModel):
#     output: Any


# dynamic_spells_model = (
#     ArcanAgent()
#     .configurable_fields(
#         user_id=ConfigurableField(
#             id="user_id",
#             name="Arcan AI User ID",
#             description=("user_id Key for Arcan AI interactions"),
#         ),
#         access_token = ConfigurableField(
#             id="token",
#             name="Arcan AI Token",
#             description=("token Key for Arcan AI interactions"),
#         )
#     )
#     .with_types(input_type=Input, output_type=Output)
# )

# add_routes(
#     app=app,
#     runnable=dynamic_spells_model,
#     per_req_config_modifier=fetch_session_from_header,
#     path="/spells",
# )

# add_routes(
#     app,
#     LLM(provider="ChatOpenAI").llm,
#     path="/openai",
#     per_req_config_modifier=fetch_session_from_header,
# )


# add_routes(
#     app,
#     LLM(provider="ChatGroq").llm,
#     per_req_config_modifier=fetch_session_from_header,
#     path="/groq",
# )

# add_routes(
#     app,
#     LLM(provider="ChatTogetherAI").llm,
#     per_req_config_modifier=fetch_session_from_header,
#     path="/together",
# )

# add_routes(
#     app,
#     runnable=LLM(provider="ChatOllama").llm,
#     per_req_config_modifier=fetch_session_from_header,
#     path="/ollama",
# )


# @app.post("/token")
# async def login_for_access_token(
#     form_data: Annotated[OAuth2PasswordRequestForm, Depends()],
#     # session: Session = Depends(session_scope()),
# ) -> TokenModel:
#     user_repo = UserRepository()
#     user_interface = UserService(user_repository=user_repo, pwd_context=pwd_context)
#     print(form_data.username, form_data.password)
#     print(user_interface)
#     user = user_interface.authenticate_user(form_data.username, form_data.password)
#     print(user)
#     if not user:
#         raise HTTPException(
#             status_code=status.HTTP_401_UNAUTHORIZED,
#             detail="Incorrect username or password",
#             headers={"WWW-Authenticate": "Bearer"},
#         )
#     access_token_expires = timedelta(minutes=ACCESS_TOKEN_EXPIRE_MINUTES)
#     access_token = user_interface.create_access_token(
#         data={"sub": user.username}, expires_delta=access_token_expires
#     )
#     return TokenModel(
#         access_token=access_token,
#         token_type="bearer",
#         user_id=user.username,
#         user=user,
#     )




# # async def UserService.get_current_active_user_from_request(
# #     request: Request, session: Session = Depends(session_scope)
# # ) -> UserModel:
# #     """Get the current active user from the request."""
# #     user_repo = UserRepository()
# #     # print(request.headers)
# #     user_interface = UserService(user_repository=user_repo, pwd_context=pwd_context)
# #     token = await oauth2_scheme(request)
# #     print(token)
# #     user = user_interface.get_current_user(token=token)
# #     print(user)
# #     if not user:
# #         raise HTTPException(
# #             status_code=status.HTTP_401_UNAUTHORIZED,
# #             detail="Invalid authentication credentials",
# #             headers={"WWW-Authenticate": "Bearer"},
# #         )
# #     # if user.disabled:
# #     # raise HTTPException(status_code=400, detail="Inactive user")
# #     return user


# # @app.get("/users/me/", response_model=UserModel)
# # async def read_users_me(
# #     current_user: Annotated[UserModel, Depends(UserService.get_current_active_user_from_request)],
# # ):
# #     return current_user


# # add_routes(
# #     app,
# #     get_per_user_retriever(vectorstore=pgVectorStore().get_vector_store()),
# #     per_req_config_modifier=per_req_config_modifier,
# #     enabled_endpoints=["invoke"],
# # )

# # %%

# # def create_session_factory(
# #     base_dir: Union[str, Path],
# # ) -> Callable[[str], BaseChatMessageHistory]:
# #     """Create a factory that can retrieve chat histories.

# #     The chat histories are keyed by user ID and conversation ID.

# #     Args:
# #         base_dir: Base directory to use for storing the chat histories.

# #     Returns:
# #         A factory that can retrieve chat histories keyed by user ID and conversation ID.
# #     """
# #     base_dir_ = Path(base_dir) if isinstance(base_dir, str) else base_dir
# #     if not base_dir_.exists():
# #         base_dir_.mkdir(parents=True)

# #     def get_chat_history(user_id: str, conversation_id: str) -> FileChatMessageHistory:
# #         """Get a chat history from a user id and conversation id."""
# #         if not _is_valid_identifier(user_id):
# #             raise ValueError(
# #                 f"User ID {user_id} is not in a valid format. "
# #                 "User ID must only contain alphanumeric characters, "
# #                 "hyphens, and underscores."
# #                 "Please include a valid cookie in the request headers called 'user-id'."
# #             )
# #         if not _is_valid_identifier(conversation_id):
# #             raise ValueError(
# #                 f"Conversation ID {conversation_id} is not in a valid format. "
# #                 "Conversation ID must only contain alphanumeric characters, "
# #                 "hyphens, and underscores. Please provide a valid conversation id "
# #                 "via config. For example, "
# #                 "chain.invoke(.., {'configurable': {'conversation_id': '123'}})"
# #             )

# #         user_dir = base_dir_ / user_id
# #         if not user_dir.exists():
# #             user_dir.mkdir(parents=True)
# #         file_path = user_dir / f"{conversation_id}.json"
# #         return FileChatMessageHistory(str(file_path))

# #     return get_chat_history


# # def _per_request_config_modifier(
# #     config: Dict[str, Any], request: Request
# # ) -> Dict[str, Any]:
# #     """Update the config"""
# #     config = config.copy()
# #     configurable = config.get("configurable", {})
# #     # Look for a cookie named "user_id"
# #     user_id = request.cookies.get("user_id", None)

# #     if user_id is None:
# #         raise HTTPException(
# #             status_code=400,
# #             detail="No user id found. Please set a cookie named 'user_id'.",
# #         )

# #     configurable["user_id"] = user_id
# #     config["configurable"] = configurable
# #     return config


# # # Declare a chain
# # prompt = ChatPromptTemplate.from_messages(
# #     [
# #         ("system", "You're an assistant by the name of Bob."),
# #         MessagesPlaceholder(variable_name="history"),
# #         ("human", "{human_input}"),
# #     ]
# # )

# # chain = prompt | ChatOpenAI()


# # class InputChat(TypedDict):
# #     """Input for the chat endpoint."""

# #     human_input: str
# #     """Human input"""


# # chain_with_history = RunnableWithMessageHistory(
# #     chain,
# #     create_session_factory("chat_histories"),
# #     input_messages_key="human_input",
# #     history_messages_key="history",
# #     history_factory_config=[
# #         ConfigurableFieldSpec(
# #             id="user_id",
# #             annotation=str,
# #             name="User ID",
# #             description="Unique identifier for the user.",
# #             default="",
# #             is_shared=True,
# #         ),
# #         ConfigurableFieldSpec(
# #             id="conversation_id",
# #             annotation=str,
# #             name="Conversation ID",
# #             description="Unique identifier for the conversation.",
# #             default="",
# #             is_shared=True,
# #         ),
# #     ],
# # ).with_types(input_type=InputChat)


# # add_routes(
# #     app,
# #     chain_with_history,
# #     per_req_config_modifier=_per_request_config_modifier,
# #     # Disable playground and batch
# #     # 1) Playground we're passing information via headers, which is not supported via
# #     #    the playground right now.
# #     # 2) Disable batch to avoid users being confused. Batch will work fine
# #     #    as long as users invoke it with multiple configs appropriately, but
# #     #    without validation users are likely going to forget to do that.
# #     #    In addition, there's likely little sense in support batch for a chatbot.
# #     disabled_endpoints=["playground", "batch"],
# #     path="/chain_with_history",
# # )


# # def _per_request_session_modifier(
# #     config: Dict[str, Any], request: Request
# # ) -> Dict[str, Any]:
# #     """Update the config"""
# #     config = config.copy()
# #     configurable = config.get("configurable", {})
# #     # Look for a cookie named "user_id"
# #     user_id = request.cookies.get("user_id", None)

# #     if user_id is None:
# #         raise HTTPException(
# #             status_code=400,
# #             detail="No user id found. Please set a cookie named 'user_id'.",
# #         )

# #     agent = ArcanAgent(user_id=user_id)

# #     configurable["user_id"] = user_id
# #     config["configurable"] = configurable
# #     return config, agent

# # add_routes(
# #     app,
# #     ArcanAgent(),
# #     path="/auth_spells",
# #     per_req_config_modifier=_per_request_session_modifier,
# #     # Disable playground and batch
# #     # 1) Playground we're passing information via headers, which is not supported via
# #     #    the playground right now.
# #     # 2) Disable batch to avoid users being confused. Batch will work fine
# #     #    as long as users invoke it with multiple configs appropriately, but
# #     #    without validation users are likely going to forget to do that.
# #     #    In addition, there's likely little sense in support batch for a chatbot.
# #     disabled_endpoints=["playground", "batch"],
# # )

# # %%

# if __name__ == "__main__":
#     import uvicorn

#     uvicorn.run(app, host="localhost", port=8000)

# # %%

```


## ../../arcan/forge/api/routes/auth.py

```python
#%%
import re
from datetime import datetime, timedelta
from typing import Any, Dict

from fastapi import APIRouter, Depends, HTTPException, Request, status
from fastapi.security import OAuth2PasswordRequestForm
from jose import JWTError, jwt
from passlib.context import CryptContext
from sqlalchemy.ext.asyncio import AsyncSession

from arcan.forge.config import settings
from arcan.forge.database.session import session_scope
from arcan.forge.repository.token import TokenRepository
from arcan.forge.repository.user import UserRepository
from arcan.forge.schemas.token import Token
from arcan.forge.schemas.user import User, UserCreate

router = APIRouter()

pwd_context = CryptContext(schemes=["bcrypt"], deprecated="auto", bcrypt__rounds=12,)  # Adjust rounds for security/performance tradeoff)

def hash_password(password: str) -> str:
    return pwd_context.hash(password)

def verify_password(plain_password: str, hashed_password: str) -> bool:
    return pwd_context.verify(plain_password, hashed_password)

def disable_user(self, username):
        user = self.user_repository.get_user(username)
        if user:
            user.disabled = True
            self.user_repository.update_user(user)

def enable_user(self, username):
    user = self.user_repository.get_user(username)
    if user:
        user.disabled = False
        self.user_repository.update_user(user)

def create_access_token(data: dict, expires_delta: timedelta | None = None):
    to_encode = data.copy()
    if expires_delta:
        expire = datetime.utcnow() + expires_delta
    else:
        expire = datetime.utcnow() + timedelta(minutes=15)
    to_encode.update({"exp": expire})
    encoded_jwt = jwt.encode(to_encode, settings.secret_key, algorithm=settings.algorithm)
    return encoded_jwt

@router.post("/register", response_model=User)
async def register_user(user_create: UserCreate, db: AsyncSession = Depends(session_scope)):
    user_repo = UserRepository(db)
    user = User(
        username=user_create.username,
        email=user_create.email,
        full_name=user_create.full_name,
        disabled=False,
        hashed_password=hash_password(user_create.password),
    )
    await user_repo.add_user(user)
    return user

@router.post("/token", response_model=Token)
async def login(form_data: OAuth2PasswordRequestForm = Depends(), db: AsyncSession = Depends(session_scope)):
    user_repo = UserRepository(db)
    user = await user_repo.get_user(form_data.username)
    if not user or not verify_password(form_data.password, user.hashed_password):
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail="Incorrect username or password")
    access_token_expires = timedelta(minutes=settings.access_token_expire_minutes)
    access_token = create_access_token(data={"sub": user.username}, expires_delta=access_token_expires)
    return {"access_token": access_token, "token_type": "bearer"}





# async def fetch_session_from_header(config: Dict[str, Any], req: Request, db: AsyncSession = Depends(session_scope)) -> Dict[str, Any]:
#     config = config.copy()
#     configurable = config.get("configurable", {})
    
#     if "arcanai_api_key" in req.headers:
#         # validate the key exists in the database
#         token_repo = TokenRepository(db)
#         # print(req.headers["arcanai_api_key"])
#         token = await token_repo.get_token(req.headers["arcanai_api_key"])
#             # id
#             # access_token
#             # token_type
#             # user_id
#             # user
#         print({"token": token})
#         if not token:
#             raise HTTPException(401, "Invalid Arcan AI API key")
#         # if token.expired:
#         #     raise HTTPException(401, "Expired Arcan AI API key")
        
#         if "user_id" in req.headers:
#             configurable["user_id"] = req.headers["user_id"]
#             configurable["access_token"] = req.headers["arcanai_api_key"]
#             config["configurable"] = configurable
#             print(config)
#     else:
#         raise HTTPException(401, "No Arcan AI API key provided")
#     return config


# def _is_valid_identifier(value: str) -> bool:
#     """Check if the value is a valid identifier."""
#     # Use a regular expression to match the allowed characters
#     valid_characters = re.compile(r"^[a-zA-Z0-9-_]+$")
#     return bool(valid_characters.match(value))

# %%

from typing import Any, Dict

from fastapi import Depends, HTTPException, Request
from sqlalchemy.ext.asyncio import AsyncSession

from arcan.forge.database.session import session_scope
from arcan.forge.repository.token import TokenRepository


async def fetch_session_from_header(config: Dict[str, Any], req: Request, db: AsyncSession = Depends(session_scope)) -> Dict[str, Any]:
    config = config.copy()
    configurable = config.get("configurable", {})

    if "arcanai_api_key" in req.headers:
        # validate the key exists in the database
        async with session_scope() as db:
            token_repo = TokenRepository(db)
            token = await token_repo.get_token(req.headers["arcanai_api_key"])
            if not token:
                raise HTTPException(401, "Invalid Arcan AI API key")

            if "user_id" in req.headers:
                configurable["user_id"] = req.headers["user_id"]
                configurable["access_token"] = req.headers["arcanai_api_key"]
                config["configurable"] = configurable
    else:
        raise HTTPException(401, "No Arcan AI API key provided")
    return config


```


## ../../arcan/forge/api/routes/user.py

```python
from fastapi import APIRouter, Depends

from arcan.forge.schemas.user import User
from arcan.forge.service.user import UserService

router = APIRouter()

@router.get("/me", response_model=User)
async def read_users_me(current_user: User = Depends(UserService.get_current_active_user)):
    return current_user

```


## ../../arcan/forge/api/routes/conversation.py

```python
from typing import List

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy.ext.asyncio import AsyncSession

from arcan.forge.api.routes.auth import pwd_context
from arcan.forge.database.session import session_scope
from arcan.forge.models.user import User
from arcan.forge.repository.conversation import ConversationRepository
from arcan.forge.schemas.conversation import (Conversation, ConversationCreate,
                                              ConversationUpdate)
from arcan.forge.service.user import UserService

router = APIRouter()


@router.post("/conversation/", response_model=Conversation)
async def create_conversation(conversation: ConversationCreate, db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    conversation_repo = ConversationRepository(db)
    conversation_data = Conversation(
        message=conversation.message,
        response=conversation.response,
        user_id=current_user.id,
    )
    await conversation_repo.add_conversation(conversation_data)
    return conversation_data

@router.get("/conversation/", response_model=List[Conversation])
async def get_conversation(db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    conversation_repo = ConversationRepository(db)
    conversation = await conversation_repo.get_conversation(current_user.id)
    return conversation

@router.put("/conversation/{conversation_id}", response_model=Conversation)
async def update_conversation(conversation_id: int, conversation: ConversationUpdate, db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    conversation_repo = ConversationRepository(db)
    existing_conversation = await conversation_repo.get_conversation(conversation_id)
    if not existing_conversation:
        raise HTTPException(status_code=404, detail="Conversation not found")
    if existing_conversation.user_id != current_user.id:
        raise HTTPException(status_code=403, detail="Not authorized to update this conversation")
    existing_conversation.message = conversation.message
    existing_conversation.response = conversation.response
    await conversation_repo.update_conversation(existing_conversation)
    return existing_conversation

@router.delete("/conversation/{conversation_id}", response_model=Conversation)
async def delete_conversation(conversation_id: int, db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    conversation_repo = ConversationRepository(db)
    existing_conversation = await conversation_repo.get_conversation(conversation_id)
    if not existing_conversation:
        raise HTTPException(status_code=404, detail="Conversation not found")
    if existing_conversation.user_id != current_user.id:
        raise HTTPException(status_code=403, detail="Not authorized to delete this conversation")
    await conversation_repo.delete_conversation(conversation_id)
    return {"detail": "Conversation deleted"}

```


## ../../arcan/forge/api/routes/__init__.py

```python

```


## ../../arcan/forge/api/routes/spells.py

```python
from typing import Any

from fastapi import APIRouter
from langchain_core.runnables import ConfigurableField
from langserve import add_routes
from pydantic import BaseModel

# from langchain_core import ArcanAgent
from arcan.casters.ai.agents import ArcanAgent
from arcan.forge.api.routes.auth import fetch_session_from_header

router = APIRouter()

class Input(BaseModel):
    input: str

class Output(BaseModel):
    output: Any


dynamic_spells_model = (
    ArcanAgent()
    .configurable_fields(
        user_id=ConfigurableField(
            id="user_id",
            name="Arcan AI User ID",
            description=("user_id Key for Arcan AI interactions"),
        ),
        access_token = ConfigurableField(
            id="token",
            name="Arcan AI Token",
            description=("token Key for Arcan AI interactions"),
        )
    )
    .with_types(input_type=Input, output_type=Output)
)

add_routes(
    app=router,
    runnable=dynamic_spells_model,
    per_req_config_modifier=fetch_session_from_header,
    path="/spells",
)
```


## ../../arcan/forge/api/routes/casters.py

```python

import os

from dotenv import load_dotenv
from fastapi import APIRouter
from langchain_core import __version__
from langserve import add_routes

from arcan.casters.ai.agents import ArcanAgent
from arcan.casters.ai.llm import LLM
from arcan.forge.api.routes.auth import fetch_session_from_header

router = APIRouter()

load_dotenv()


# %%
MIN_VERSION_LANGCHAIN_CORE = (0, 1, 0)

# Split the version string by "." and convert to integers
LANGCHAIN_CORE_VERSION = tuple(map(int, __version__.split(".")))

if LANGCHAIN_CORE_VERSION < MIN_VERSION_LANGCHAIN_CORE:
    raise RuntimeError(
        f"Minimum required version of langchain-core is {MIN_VERSION_LANGCHAIN_CORE}, "
        f"but found {LANGCHAIN_CORE_VERSION}"
    )


ENVIRONMENT = os.environ.get("ENVIRONMENT")
ARCANAI_API_TOKEN = os.environ.get("ARCANAI_API_TOKEN")


@router.get("/api/check")
async def index():
    return {"message": "Arcan is Running!"}

@router.get("/api/chat")
async def chat(
    user_id: str,
    query: str,
):
    if ENVIRONMENT == "cloud":
        agent = ArcanAgent(user_id=user_id)
        response = agent.invoke({"input": query})
    elif ENVIRONMENT == "local":
        agent = ArcanAgent(
            user_id=user_id,
        )
        response = agent.invoke({"input": query, "chat_history": []})
    return {"response": response}


add_routes(
    router,
    LLM(provider="ChatOpenAI").llm,
    path="/openai",
    per_req_config_modifier=fetch_session_from_header,
)


add_routes(
    router,
    LLM(provider="ChatGroq").llm,
    per_req_config_modifier=fetch_session_from_header,
    path="/groq",
)

add_routes(
    router,
    LLM(provider="ChatTogetherAI").llm,
    per_req_config_modifier=fetch_session_from_header,
    path="/together",
)

add_routes(
    router,
    runnable=LLM(provider="ChatOllama").llm,
    per_req_config_modifier=fetch_session_from_header,
    path="/ollama",
)
```


## ../../arcan/forge/api/routes/router.py

```python
from fastapi import APIRouter

from arcan.forge.api.routes import (auth, casters, chat_history, conversation,
                                    user)

base_router = APIRouter()

base_router.include_router(auth.router, tags=["auth"], prefix="/v1")
base_router.include_router(chat_history.router, tags=["chat_history"], prefix="/v1")
base_router.include_router(conversation.router, tags=["conversation"], prefix="/v1")
base_router.include_router(user.router, tags=["user"], prefix="/v1")
# base_router.include_router(spells.router, tags=["spells"], prefix="/v1")
base_router.include_router(casters.router, tags=["casters"], prefix="/v1")

```


## ../../arcan/forge/api/routes/chat_history.py

```python
from typing import List

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy.ext.asyncio import AsyncSession

from arcan.forge.database.session import session_scope
from arcan.forge.models.user import User
from arcan.forge.repository.chat_history import ChatHistoryRepository
from arcan.forge.schemas.chat_history import (ChatHistory, ChatHistoryCreate,
                                              ChatHistoryUpdate)
from arcan.forge.service.user import UserService

router = APIRouter()

@router.post("/chat_history/", response_model=ChatHistory)
async def create_chat_history(chat_history: ChatHistoryCreate, db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    chat_history_repo = ChatHistoryRepository(db)
    chat_history_data = ChatHistory(
        history=chat_history.history,
        user_id=current_user.id,
    )
    await chat_history_repo.add_chat_history(chat_history_data)
    return chat_history_data

@router.get("/chat_history/", response_model=ChatHistory)
async def get_chat_history(db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    chat_history_repo = ChatHistoryRepository(db)
    chat_history = await chat_history_repo.get_chat_history(current_user.id)
    if not chat_history:
        raise HTTPException(status_code=404, detail="Chat history not found")
    return chat_history

@router.put("/chat_history/", response_model=ChatHistory)
async def update_chat_history(chat_history: ChatHistoryUpdate, db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    chat_history_repo = ChatHistoryRepository(db)
    existing_chat_history = await chat_history_repo.get_chat_history(current_user.id)
    if not existing_chat_history:
        raise HTTPException(status_code=404, detail="Chat history not found")
    existing_chat_history.history = chat_history.history
    await chat_history_repo.update_chat_history(existing_chat_history)
    return existing_chat_history

@router.delete("/chat_history/", response_model=ChatHistory)
async def delete_chat_history(db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    chat_history_repo = ChatHistoryRepository(db)
    await chat_history_repo.delete_chat_history(current_user.id)
    return {"detail": "Chat history deleted"}

```


## ../../arcan/forge/service/user.py

```python
import os
from datetime import datetime, timedelta, timezone

from fastapi import Depends, HTTPException, status
from fastapi.security import OAuth2PasswordBearer
from jose import JWTError, jwt
from passlib.context import CryptContext

from arcan.forge.models.user import User
from arcan.forge.repository.user import UserRepository
from arcan.forge.schemas.token import TokenData
from arcan.forge.schemas.user import UserCreate

oauth2_scheme = OAuth2PasswordBearer(tokenUrl="token")

SECRET_KEY = os.environ.get("ARCANAI_API_KEY")
ALGORITHM = "HS256"
ACCESS_TOKEN_EXPIRE_MINUTES = 30

pwd_context = CryptContext(schemes=["bcrypt"], deprecated="auto", bcrypt__rounds=12,)  # Adjust rounds for security/performance tradeoff)

class UserService:
    def __init__(self, user_repository: UserRepository, pwd_context: CryptContext = pwd_context):
        self.user_repository = user_repository
        self.pwd_context = pwd_context

    def authenticate_user(self, username, password):
        user = self.user_repository.get_user(username)
        if not user:
            return False
        if not self.verify_password(password, user.hashed_password):
            return False
        return user

    def register_user(self, user_create: UserCreate):
        user = User(
            username=user_create.username,
            email=user_create.email,
            full_name=user_create.full_name,
            disabled=True,
            hashed_password=self.hash_password(user_create.password),
            created_at=datetime.now(timezone.utc),
        )
        self.user_repository.add_user(user)

    def hash_password(self, password):
        return self.pwd_context.hash(password)

    def verify_password(self, plain_password, hashed_password):
        return self.pwd_context.verify(plain_password, hashed_password)

    def disable_user(self, username):
        user = self.user_repository.get_user(username)
        if user:
            user.disabled = True
            self.user_repository.update_user(user)

    def enable_user(self, username):
        user = self.user_repository.get_user(username)
        if user:
            user.disabled = False
            self.user_repository.update_user(user)

    def create_access_token(self, data: dict, expires_delta: timedelta | None = None):
        to_encode = data.copy()
        if expires_delta:
            expire = datetime.now(timezone.utc) + expires_delta
        else:
            expire = datetime.now(timezone.utc) + timedelta(minutes=15)
        to_encode.update({"exp": expire})
        encoded_jwt = jwt.encode(to_encode, SECRET_KEY, algorithm=ALGORITHM)
        return encoded_jwt

    async def get_current_user(self, token: str = Depends(oauth2_scheme)):
        credentials_exception = HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Could not validate credentials",
            headers={"WWW-Authenticate": "Bearer"},
        )
        try:
            payload = jwt.decode(token, SECRET_KEY, algorithms=[ALGORITHM])
            username: str = payload.get("sub")
            if username is None:
                raise credentials_exception
            token_data = TokenData(username=username)
        except JWTError:
            raise credentials_exception
        user = self.user_repository.get_user(username)
        if user is None:
            raise credentials_exception
        return user

    async def get_current_active_user(self, current_user: User = Depends(get_current_user)):
        if current_user.disabled:
            raise HTTPException(status_code=400, detail="Inactive user")
        return current_user

```


## ../../arcan/forge/service/__init__.py

```python

```


## ../../arcan/forge/entities/__init__.py

```python

```


## ../../arcan/spells/scrapping.py

```python
# %%
import json
import os
import time

import html2text
import requests
from bs4 import BeautifulSoup
from dotenv import load_dotenv
from firecrawl import FirecrawlApp
from langchain.agents import Tool
from langchain_community.tools import WikipediaQueryRun
from langchain_community.tools.tavily_search import TavilySearchResults
from langchain_community.utilities import WikipediaAPIWrapper
from pydantic import AnyHttpUrl, FilePath
from selenium import webdriver
from selenium.webdriver.chrome.options import Options

brwoserless_api_key = os.getenv("BROWSERLESS_API_KEY")


def scrape_website(url: str):
    # scrape website, and also will summarize the content based on objective if the content is too large
    # objective is the original objective & task that user give to the agent, url is the url of the website to be scraped

    print("Scraping website...")
    # Define the headers for the request
    headers = {
        "Cache-Control": "no-cache",
        "Content-Type": "application/json",
    }

    # Define the data to be sent in the request
    data = {"url": url}

    # Convert Python object to JSON string
    data_json = json.dumps(data)

    # Send the POST request
    response = requests.post(
        f"https://chrome.browserless.io/content?token={brwoserless_api_key}",
        headers=headers,
        data=data_json,
        timeout=60,
    )

    # Check the response status code
    if response.status_code == 200:
        soup = BeautifulSoup(response.content, "html.parser")
        text = soup.get_text()
        if len(text) < 100:
            raise Exception("Content too short")
        return text
    else:
        raise Exception(f"HTTP request failed with status code {response.status_code}")


def scrape_website_selenium(url):
    try:
        # Configure Selenium with a headless browser
        options = Options()
        options.headless = True
        driver = webdriver.Chrome(options=options)

        # Access the webpage
        driver.get(url)

        # Wait for JavaScript to render. Adjust time as needed.
        time.sleep(5)  # Time in seconds

        # Extract the page source
        page_source = driver.page_source

        # Close the browser
        driver.quit()

        # Convert HTML to Markdown
        converter = html2text.HTML2Text()
        markdown = converter.handle(page_source)
        if len(markdown) < 100:
            raise Exception("Content too short")

        return markdown
    except Exception as e:
        print(f"Error scraping website: {e}")
        raise e


import os
import re
from pathlib import Path

import httpx
from bs4 import BeautifulSoup


def scrape_url(url) -> str:
    # fetch article; simulate desktop browser
    headers = {
        "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_11_2) AppleWebKit/601.3.9 (KHTML, like Gecko) Version/9.0.2 Safari/601.3.9"
    }
    response = httpx.get(url, headers=headers)
    soup = BeautifulSoup(response.text, "lxml")

    for tag in soup.find_all():
        if tag.string:
            stripped_string = tag.string.strip()
            tag.string.replace_with(stripped_string)

    text = soup.get_text()
    clean_text = text.replace("\n\n", "\n")

    return clean_text.replace("\t", "")


def url_text_scrapper(url: str):
    domain_regex = r"(?:https?:\/\/)?(?:[^@\n]+@)?(?:www\.)?([^:\/\n\.]+)"

    match = re.search(domain_regex, url)

    if match:
        domain = match.group(1)
        clean_domain = re.sub(r"[^a-zA-Z0-9]+", "", domain)

    # Support caching speech text on disk.
    file_path = Path(f"scrappings/{clean_domain}.txt")
    print(file_path)

    if file_path.exists():
        scrapped_text = file_path.read_text()
    else:
        print("Scrapping from url")
        scrapped_text = scrape_url(url)
        os.makedirs(file_path.parent, exist_ok=True)
        file_path.write_text(scrapped_text)

    return scrapped_text, clean_domain


def firecrawl_loader(url: str, mode: str = "scrape"):
    from langchain_community.document_loaders import FireCrawlLoader

    loader = FireCrawlLoader(
        api_key=os.environ.get("FIRECRAWL_API_KEY"),
        url=url,
        mode=mode,  # scrape: Scrape single url and return the markdown.
        # crawl: Crawl the url and all accessible sub pages and return the markdown for each one.
    )
    return loader


def firecrawl_scrape(url):
    """
    The function `firecrawl_scrape` takes a URL as input and uses the FirecrawlApp class to scrape the
    content of the webpage at that URL.

    :param url: The `url` parameter in the `firecrawl_scrape` function is a string that represents the
    URL of the webpage that you want to scrape using the FirecrawlApp
    :return: The `firecrawl_scrape` function is returning the result of calling the `scrape_url` method
    of a `FirecrawlApp` instance with the provided `url` as an argument. It is a markdown string of the
    scraped content of the webpage at the provided URL.
    """
    return FirecrawlApp().scrape_url(
        url,
        {
            "extractorOptions": {
                "mode": "llm-extraction",
                "extractionPrompt": "Extract the key elements, segment by NER, and summarize the content. Make sure the returned content is at most 16385 tokens",
            },
            "pageOptions": {"onlyMainContent": True},
        },
    )
    return FirecrawlApp().scrape_url(
        url,
        {
            "extractorOptions": {
                "mode": "llm-extraction",
                "extractionPrompt": "Extract the key elements, segment by NER, and summarize the content. Make sure the returned content is at most 16385 tokens",
            },
            "pageOptions": {"onlyMainContent": True},
        },
    )


# from pydantic import AnyHttpUrl, FilePath

# def scrapegraph_scrape(url: AnyHttpUrl, prompt: str):
#     from scrapegraphai.graphs import SmartScraperGraph


#     graph_config = {
#         "llm": {
#             "model": "ollama/mistral",
#             "temperature": 0,
#             "format": "json",  # Ollama needs the format to be specified explicitly
#             "base_url": "http://localhost:11434",  # set Ollama URL
#         },
#         "embeddings": {
#             "model": "ollama/nomic-embed-text",
#             "base_url": "http://localhost:11434",  # set Ollama URL
#         },
#         "verbose": True,
#     }

#     smart_scraper_graph = SmartScraperGraph(
#         prompt=prompt,
#         # also accepts a string with the already downloaded HTML code
#         source=url.__str__(),
#         config=graph_config,
#         prompt=prompt,
#         # also accepts a string with the already downloaded HTML code
#         source=url.__str__(),
#         config=graph_config,
#     )

#     result = smart_scraper_graph.run()
#     print(result)


async def llama_parse_scrape(pdf_path: FilePath):
    import nest_asyncio

    nest_asyncio.apply()

    from llama_parse import LlamaParse

    parser = LlamaParse(
        api_key=os.environ.get("LLAMA_CLOUD_API_KEY"),
        result_type="markdown",  # "markdown" and "text" are available
        num_workers=4,  # if multiple files passed, split in `num_workers` API calls
        verbose=True,
        language="en",  # Optionally you can define a language, default=en
    )

    # async
    documents = await parser.aload_data(pdf_path)
    return documents

```


## ../../arcan/spells/__init__.py

```python
from abc import ABC, abstractmethod


class SpellCommand(ABC):
    @abstractmethod
    def execute(self):
        pass

class ScrappingSpell(SpellCommand):
    def execute(self):
        # Scrapping logic here
        pass

class SearchSpell(SpellCommand):
    def execute(self):
        # Search logic here
        pass

# Usage
def run_spell(spell: SpellCommand):
    spell.execute()

spell = ScrappingSpell()
run_spell(spell)

```


## ../../arcan/spells/self.py

```python
# includes the tools/spells that are self-targeted. Defines who the caster is

# Path: arcan/spells/self.py
#%%



import fnmatch
import os
import re
import subprocess
from pathlib import Path


def read_gitignore(root_dir):
    ignore_patterns = []
    gitignore_path = Path(root_dir) / '.gitignore'
    if gitignore_path.exists():
        with open(gitignore_path, 'r') as f:
            ignore_patterns = [line.strip() for line in f.readlines() if line.strip() and not line.startswith('#')]
    return ignore_patterns

def should_ignore(file_path, ignore_patterns):
    return any(fnmatch.fnmatch(file_path, pattern) for pattern in ignore_patterns)

def remove_ansi_escape_codes(text):
    ansi_escape_pattern = re.compile(r'\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])')
    return ansi_escape_pattern.sub('', text)

def clean_output(text):
    # Remove ANSI escape codes
    ansi_escape_pattern = re.compile(r'\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])')
    cleaned_text = ansi_escape_pattern.sub('', text)
    # Replace non-breaking spaces with regular spaces
    cleaned_text = cleaned_text.replace('\xa0', ' ')
    return cleaned_text

def get_code(root_dir, output_md_path):
    def write_and_append(file, lst, content):
        try:
            file.write(content)
        except AttributeError as e:
            print(e)
        lst.append(content)

    ignore_patterns = read_gitignore(root_dir)
    # Convert ignore patterns suitable for the tree command
    tree_ignore_patterns = ','.join(ignore_patterns).replace('*', '') + ',*.pyc'
    code = []
    with open(output_md_path, 'w') as md_file:
        # Adding tree command output to the markdown file with filters
        try:
            tree_command = ['tree', root_dir, '-I', tree_ignore_patterns]
            tree_output = subprocess.check_output(tree_command, universal_newlines=True)
            cleaned_tree_output = clean_output(tree_output)
            write_and_append(md_file, code, f"\n## Directory Structure\n\n```\n{cleaned_tree_output}\n```\n")
        except FileNotFoundError:
            write_and_append(md_file, code, "\n## Directory Structure\n\n```\nTree command not available.\n```\n")
        except subprocess.CalledProcessError as e:
            write_and_append(md_file, code, f"\n## Directory Structure\n\n```\nError executing tree command: {e}\n```\n")


        for root, dirs, files in os.walk(root_dir):
            for file in files:
                file_path = Path(root) / file
                relative_file_path = file_path.relative_to(root_dir)
                if file.endswith('.py') and not file.endswith('.pyc') and not 'cpython' in file and not should_ignore(str(relative_file_path), ignore_patterns):
                    content_header = f"\n## {file_path}\n\n```python\n"
                    write_and_append(md_file, code, content_header)
                    with open(file_path, 'r') as py_file:
                        file_content = py_file.read()
                        write_and_append(md_file, code, file_content)
                    content_footer = "\n```\n\n"
                    write_and_append(md_file, code, content_footer)
    return code



def get_knowledge(caster):
    return [
        understanding for understanding in (
            [
                get_code(root_dir='../../', output_md_path='self_code.md'),
                # identity,
                # values,
                # beliefs,
                # desires,
                # intentions,
                # emotions,
                # thoughts,
                # memories,
                # experiences,
                # skills,
                # abilities,
                # powers,
                # strengths,
                # weaknesses,
                # limitations,
                # knowledge,
                # wisdom,
                # understanding,
                # intelligence,
                # intuition,
                # creativity,
                # imagination,
                # perception,
                # awareness,
                # consciousness,
                # subconsciousness,
                # unconsciousness,
                # self,
            ]
        )
    ]


def knowledge(caster):
    return get_knowledge(caster)

# %%

```


## ../../arcan/spells/vector_search.py

```python
# %%
import os
from typing import Any, Dict, List, Optional, Union

import pandas as pd
from fastapi import Depends, FastAPI, HTTPException, Request, status
from fastapi.security import OAuth2PasswordBearer, OAuth2PasswordRequestForm
from langchain.document_loaders import (DataFrameLoader,
                                        UnstructuredMarkdownLoader)
from langchain.embeddings.openai import OpenAIEmbeddings
from langchain.text_splitter import RecursiveCharacterTextSplitter
from langchain.vectorstores import FAISS, Chroma, VectorStore
from langchain_community.document_loaders import TextLoader
from langchain_community.document_loaders.base import BaseLoader
from langchain_community.vectorstores import SupabaseVectorStore
from langchain_community.vectorstores.chroma import Chroma
from langchain_core.documents import Document
from langchain_core.runnables import (ConfigurableField, RunnableConfig,
                                      RunnableSerializable)
from langchain_core.vectorstores import VectorStore
from langchain_openai import OpenAIEmbeddings
from langchain_text_splitters import CharacterTextSplitter
from langserve import add_routes
from langserve.pydantic_v1 import BaseModel
from supabase.client import Client, create_client
from typing_extensions import Annotated

embeddings = OpenAIEmbeddings()


class VectorStoreHandler:
    def __init__(self, **kwargs):
        self.kwargs = kwargs

    def get_vectorstore(self):
        get_vectorstore_strategies = {
            "chroma": load_chroma_vectorstore,
            "faiss": load_faiss_vectorstore,
        }
        vectorstore_strategy = self.kwargs.get("vectorstore", "chroma")
        return get_vectorstore_strategies[vectorstore_strategy]()

    def set_vectorstore(self):
        set_vectorstore_strategies = {
            "chroma": pandas_df_vectorstore_loader,
            "faiss": faiss_metadata_index_loader,
        }
        vectorstore_strategy = self.kwargs.get("vectorstore", "chroma")
        return set_vectorstore_strategies[vectorstore_strategy]()


def load_chroma_vectorstore():
    return Chroma(
        persist_directory="indexes/croma_index", embedding_function=embeddings
    )


def load_faiss_vectorstore(index_key: str = "default"):
    return FAISS.load_local(f"indexes/faiss_index/{index_key}", embeddings)


def faiss_text_index_loader(text: str, index_key: str = "default"):
    text_splitter = RecursiveCharacterTextSplitter(chunk_size=1000, chunk_overlap=20)
    texts = text_splitter.split_text(text)

    docsearch = FAISS.from_texts(
        texts,
        OpenAIEmbeddings(chunk_size=500),
        metadatas=[{"source": i} for i in range(len(texts))],
    )
    docsearch.save_local(f"indexes/faiss_index/{index_key}")
    return docsearch


def faiss_metadata_index_loader(
    metadata_path: str = "indexes/metadata/schema.md",
):
    loader = UnstructuredMarkdownLoader(metadata_path)
    data = loader.load()
    # df = pd.read_csv(data_path)
    text_splitter = RecursiveCharacterTextSplitter(chunk_size=1000, chunk_overlap=20)
    texts = text_splitter.split_documents(data)

    # df_loader = DataFrameLoader(df, page_content_column=page_content_column)
    # docs = df_loader.load()

    faiss_store = FAISS.from_documents(texts, embeddings)
    # docsearch.add_documents(docs)
    faiss_store.save_local("indexes/faiss_index")

    # with open("vectors.pkl", "wb") as f:
    #     pickle.dump(docsearch, f)


def pandas_df_vectorstore_loader(
    data_path: str = "indexes/samples/telemetry_sample_forecast.csv",
    page_content_column: str = "y",
):
    df = pd.read_csv(data_path)
    # jdf = df.to_dict(orient='split')
    loader = DataFrameLoader(df, page_content_column=page_content_column)
    docs = loader.load()

    # VectorStoreRetrieverMemory

    vectorstore_ts = Chroma.from_documents(
        docs, embeddings, persist_directory="croma_index"
    )
    # docs = pandas_df_vectorstore_loader(data_path=df_path,  page_content_column=data_columnn)
    vectorstore_ts.persist()

    return docs


# -- Enable the pgvector extension to work with embedding vectors
# create extension if not exists vector;

# -- Create a table to store your documents
# create table
#   documents (
#     id uuid primary key,
#     content text, -- corresponds to Document.pageContent
#     metadata jsonb, -- corresponds to Document.metadata
#     embedding vector (1536) -- 1536 works for OpenAI embeddings, change if needed
#   );

# -- Create a function to search for documents
# create function match_documents (
#   query_embedding vector (1536),
#   filter jsonb default '{}'
# ) returns table (
#   id uuid,
#   content text,
#   metadata jsonb,
#   similarity float
# ) language plpgsql as $$
# #variable_conflict use_column
# begin
#   return query
#   select
#     id,
#     content,
#     metadata,
#     1 - (documents.embedding <=> query_embedding) as similarity
#   from documents
#   where metadata @> filter
#   order by documents.embedding <=> query_embedding;
# end;
# $$;

# %%


class pgVectorStore:
    def __init__(
        self, table_name: str = "documents", query_name: str = "match_documents"
    ):
        supabase_url = os.environ.get("SUPABASE_URL")
        supabase_key = os.environ.get("SUPABASE_SERVICE_KEY")
        self.supabase: Client = create_client(supabase_url, supabase_key)
        self.embeddings = OpenAIEmbeddings()
        self.table_name = table_name
        self.query_name = query_name
        self.vector_store = self.get_vector_store()

    def get_vector_store(self) -> VectorStore:
        return SupabaseVectorStore(
            embedding=self.embeddings,
            client=self.supabase,
            table_name=self.table_name,
            query_name=self.query_name,
        )

    def read(self, query):
        matched_docs = self.vector_store.similarity_search(query)
        return matched_docs[0].page_content

    def write(
        self,
        loader: BaseLoader,
        chunk_size: int = 1000,
        chunk_overlap: int = 80,
    ):
        documents = loader.load()
        text_splitter = CharacterTextSplitter(
            chunk_size=chunk_size, chunk_overlap=chunk_overlap
        )
        docs = text_splitter.split_documents(documents)
        self.vector_store.from_documents(
            docs,
            self.embeddings,
            client=self.supabase,
            table_name=self.table_name,
            query_name=self.query_name,
            chunk_size=chunk_size,
        )


# %%

# vec = VectorStore()
# loader = firecrawl_loader('https://python.langchain.com/v0.1/docs/integrations/vectorstores/supabase/')
# vec.write(loader)


class PerUserVectorstore(RunnableSerializable):
    """A custom runnable that returns a list of documents for the given user.

    The runnable is configurable by the user, and the search results are
    filtered by the user ID.
    """

    user_id: Optional[str]
    vectorstore: VectorStore

    class Config:
        # Allow arbitrary types since VectorStore is an abstract interface
        # and not a pydantic model
        arbitrary_types_allowed = True

    def _invoke(
        self, input: str, config: Optional[RunnableConfig] = None, **kwargs: Any
    ) -> List[Document]:
        """Invoke the retriever."""
        # WARNING: Verify documentation of underlying vectorstore to make
        # sure that it actually uses filters.
        # Highly recommended to use unit-tests to verify this behavior, as
        # implementations can be different depending on the underlying vectorstore.
        # retriever = self.vectorstore.as_retriever(
        #     search_kwargs={"filter": {"owner_id": self.user_id}}
        # )
        # return retriever.invoke(input, config=config)
        matched_docs = self.vector_store.similarity_search(input)
        return matched_docs[0].page_content

    def invoke(
        self, input: str, config: Optional[RunnableConfig] = None, **kwargs
    ) -> List[Document]:
        """Add one to an integer."""
        return self._call_with_config(self._invoke, input, config, **kwargs)


async def per_req_config_modifier(config: Dict, request: Request) -> Dict:
    from arcan.forge.service.user import UserService

    """Modify the config for each request."""
    user = await UserService.get_current_active_user(request)
    config["configurable"] = {}
    # Attention: Make sure that the user ID is over-ridden for each request.
    # We should not be accepting a user ID from the user in this case!
    config["configurable"]["user_id"] = user.username
    return config


def get_per_user_retriever(vectorstore: VectorStore, user_id: str = None):
    per_user_retriever = PerUserVectorstore(
        user_id=user_id,
        vectorstore=vectorstore,
    ).configurable_fields(
        # Attention: Make sure to override the user ID for each request in the
        # per_req_config_modifier. This should not be client configurable.
        user_id=ConfigurableField(
            id="user_id",
            name="User ID",
            description="The user ID to use for the retriever.",
        )
    )
    return per_user_retriever


# %%

```


## ../../arcan/spells/search.py

```python
# %%
import json
import os

import requests

serper_api_key = os.getenv("SERP_API_KEY")


def serper_api_search(query):
    url = "https://google.serper.dev/search"
    payload = json.dumps({"q": query})
    headers = {"X-API-KEY": serper_api_key, "Content-Type": "application/json"}

    response = requests.request("POST", url, headers=headers, data=payload)
    print(response.text)
    return response.text

```

