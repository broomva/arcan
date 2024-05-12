# %%
from langchain.prompts import ChatPromptTemplate
from langchain_core.runnables import Runnable
from langserve import RemoteRunnable


class RunnableFactory:
    def __init__(self, base_url: str = "http://localhost:8000/"):
        self.base_url = base_url
        self.runnable_cache = {}

    def get_runnable(self, runnable_name: str, cache: bool = True) -> Runnable:
        if cache and runnable_name in self.runnable_cache:
            return self.runnable_cache[runnable_name]

        runnable = RemoteRunnable(self.base_url + runnable_name + "/")
        if cache:
            self.runnable_cache[runnable_name] = runnable
        return runnable


class ArcanRunnables:
    def __init__(self, base_url: str = "http://localhost:8000/"):
        self.factory = RunnableFactory(base_url=base_url)

    def get_chat_spells_agent_runnable(self):
        return self.factory.get_runnable(runnable_name="spells_agent")

    def get_openai_runnable(self):
        return self.factory.get_runnable(runnable_name="openai")

    def get_groq_runnable(self):
        return self.factory.get_runnable(runnable_name="groq")


# %%


# from langchain.schema import HumanMessage, SystemMessage
# from langchain.schema.runnable import RunnableMap

# arcan_runnables = ArcanRunnables(base_url="http://localhost:8000/")
# chat_spells_agent = arcan_runnables.get_chat_spells_agent_runnable()
# openai_runnable = arcan_runnables.get_openai_runnable()
# groq_runnable = arcan_runnables.get_groq_runnable()


# prompt = ChatPromptTemplate.from_messages(
#     [("system", "Tell me a long story about {topic}")]
# )

# # Can define custom chains
# chain = prompt | RunnableMap({
#     "openai": openai_runnable,
#     "groq": groq_runnable,
# })
# # %%

# chain.batch([{"topic": "parrots"}, {"topic": "cats"}])


# %%
