

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