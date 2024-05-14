import os

import pytest
from fastapi.testclient import TestClient
from httpx import AsyncClient
from sqlalchemy.orm import Session

from arcan.api import app  # Adjust this import based on your project structure
from arcan.api.datamodel.engine import session_scope
from arcan.api.session import ArcanSession


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
@patch("arcan.api.datamodel.engine")  # Correct the import path as necessary
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
    mock_token.credentials = os.getenv("ARCAN_API_KEY")

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
