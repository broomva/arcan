# %%

import os
from typing import Any, Callable, Dict, List, Optional, Union

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
