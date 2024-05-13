from typer import Typer, echo

cli = Typer()

__version__ = "0.1.0"


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
    from arcan.ai.chains import ArcanConversationChain
    from arcan.spells.scrapping import url_text_scrapper
    from arcan.spells.vector_search import (
        faiss_text_index_loader,
        load_faiss_vectorstore,
    )

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
    from arcan.ai.agents import ArcanConversationAgent, agent_chat

    agent = ArcanConversationAgent().agent
    return await agent_chat(query, agent)
