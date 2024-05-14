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
from langchain.agents import (
    AgentExecutor,
    AgentType,
    create_tool_calling_agent,
    initialize_agent,
    load_tools,
)
from langchain.agents.agent_types import AgentType
from langchain.agents.format_scratchpad.openai_tools import (
    format_to_openai_tool_messages,
)
from langchain.agents.format_scratchpad.tools import format_to_tool_messages
from langchain.agents.output_parsers.tools import ToolsAgentOutputParser
from langchain.embeddings.openai import OpenAIEmbeddings
from langchain.memory import ConversationBufferMemory
from langchain.pydantic_v1 import BaseModel
from langchain.sql_database import SQLDatabase
from langchain_community.agent_toolkits import FileManagementToolkit, SQLDatabaseToolkit
from langchain_core.callbacks import CallbackManagerForChainRun
from langchain_core.load.serializable import (
    Serializable,
    SerializedConstructor,
    SerializedNotImplemented,
)
from langchain_core.messages import AIMessage, HumanMessage, SystemMessage
from langchain_core.prompts import ChatPromptTemplate

# from langchain_core.pydantic_v1 import BaseModel
from langchain_core.runnables import (
    ConfigurableField,
    ConfigurableFieldSpec,
    Runnable,
    RunnableConfig,
    RunnablePassthrough,
    RunnableSerializable,
)
from langchain_core.runnables.base import Runnable, RunnableBindingBase
from langchain_core.runnables.utils import (
    AddableDict,
    AnyConfigurableField,
    ConfigurableField,
    ConfigurableFieldSpec,
    Input,
    Output,
    create_model,
    get_unique_config_specs,
)
from langchain_openai import ChatOpenAI, OpenAIEmbeddings
from pydantic import BaseModel, Field
from sqlalchemy.dialects.postgresql import insert
from sqlalchemy.exc import SQLAlchemyError
from sqlalchemy.orm import Session

from arcan.ai.agents.helpers import AsyncIteratorCallbackHandler
from arcan.ai.agents.session import ArcanSession
from arcan.ai.llm import LLM
from arcan.ai.parser import ArcanOutputParser
from arcan.ai.prompts import arcan_prompt, spells_agent_prompt

# from arcan.ai.router import semantic_layer
from arcan.ai.tools import tools as spells

# from arcan.api.session import ArcanSession
# from arcan.ai.agents import ArcanAgent
from arcan.datamodel.chat_history import ChatHistory
from arcan.datamodel.conversation import Conversation
from arcan.datamodel.engine import session_scope


class ArcanAgent(RunnableSerializable):
    tools: List = Field(default_factory=list)
    bare_tools: List = Field(default_factory=list)
    agent_tools: List = Field(default_factory=list)
    agent_type: str = "arcan_spells_agent"
    chat_history: List = Field(default_factory=list)
    user_id: Optional[str] = None
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
        verbose: bool = False,
        configs: list = [],
        **kwargs,
    ):
        super().__init__(
            tools=tools,
            agent_type=agent_type,
            chat_history=chat_history,
            user_id=user_id,
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
        self, user_id: str, provided_agent: ArcanAgent = None
    ) -> ArcanAgent:
        """
        Retrieves or creates a ArcanAgent for a given user_id.

        :param user_id: The unique identifier for the user.
        :return: An instance of ArcanAgent.
        """
        if provided_agent is None:
            agent = self.session.agents.get(user_id)
            chat_history = []

            # Obtain a new database session
            try:
                chat_history = self.session.get_chat_history(user_id)
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
                user_id=self.user_id, agent_history=self.chat_history
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
