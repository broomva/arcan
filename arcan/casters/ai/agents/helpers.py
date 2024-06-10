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
