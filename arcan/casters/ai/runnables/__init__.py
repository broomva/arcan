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
