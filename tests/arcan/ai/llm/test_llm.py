import os

import pytest
from dotenv import load_dotenv

from arcan.ai.llm import LLM, ChatGroq, ChatOpenAI, LLMFactory, OpenAI

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