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
