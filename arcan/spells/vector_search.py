# %%
import os
from typing import Any, Dict, List, Optional, Union

import pandas as pd
from fastapi import Depends, FastAPI, HTTPException, Request, status
from fastapi.security import OAuth2PasswordBearer, OAuth2PasswordRequestForm
from langchain.document_loaders import DataFrameLoader, UnstructuredMarkdownLoader
from langchain.embeddings.openai import OpenAIEmbeddings
from langchain.text_splitter import RecursiveCharacterTextSplitter
from langchain.vectorstores import FAISS, Chroma, VectorStore
from langchain_community.document_loaders import TextLoader
from langchain_community.document_loaders.base import BaseLoader
from langchain_community.vectorstores import SupabaseVectorStore
from langchain_community.vectorstores.chroma import Chroma
from langchain_core.documents import Document
from langchain_core.runnables import (
    ConfigurableField,
    RunnableConfig,
    RunnableSerializable,
)
from langchain_core.vectorstores import VectorStore
from langchain_openai import OpenAIEmbeddings
from langchain_text_splitters import CharacterTextSplitter
from langserve import add_routes
from langserve.pydantic_v1 import BaseModel
from supabase.client import Client, create_client
from typing_extensions import Annotated

embeddings = OpenAIEmbeddings()


class VectorStoreHandler:
    def __init__(self, **kwargs):
        self.kwargs = kwargs

    def get_vectorstore(self):
        get_vectorstore_strategies = {
            "chroma": load_chroma_vectorstore,
            "faiss": load_faiss_vectorstore,
        }
        vectorstore_strategy = self.kwargs.get("vectorstore", "chroma")
        return get_vectorstore_strategies[vectorstore_strategy]()

    def set_vectorstore(self):
        set_vectorstore_strategies = {
            "chroma": pandas_df_vectorstore_loader,
            "faiss": faiss_metadata_index_loader,
        }
        vectorstore_strategy = self.kwargs.get("vectorstore", "chroma")
        return set_vectorstore_strategies[vectorstore_strategy]()


def load_chroma_vectorstore():
    return Chroma(
        persist_directory="indexes/croma_index", embedding_function=embeddings
    )


def load_faiss_vectorstore(index_key: str = "default"):
    return FAISS.load_local(f"indexes/faiss_index/{index_key}", embeddings)


def faiss_text_index_loader(text: str, index_key: str = "default"):
    text_splitter = RecursiveCharacterTextSplitter(chunk_size=1000, chunk_overlap=20)
    texts = text_splitter.split_text(text)

    docsearch = FAISS.from_texts(
        texts,
        OpenAIEmbeddings(chunk_size=500),
        metadatas=[{"source": i} for i in range(len(texts))],
    )
    docsearch.save_local(f"indexes/faiss_index/{index_key}")
    return docsearch


def faiss_metadata_index_loader(
    metadata_path: str = "indexes/metadata/schema.md",
):
    loader = UnstructuredMarkdownLoader(metadata_path)
    data = loader.load()
    # df = pd.read_csv(data_path)
    text_splitter = RecursiveCharacterTextSplitter(chunk_size=1000, chunk_overlap=20)
    texts = text_splitter.split_documents(data)

    # df_loader = DataFrameLoader(df, page_content_column=page_content_column)
    # docs = df_loader.load()

    faiss_store = FAISS.from_documents(texts, embeddings)
    # docsearch.add_documents(docs)
    faiss_store.save_local("indexes/faiss_index")

    # with open("vectors.pkl", "wb") as f:
    #     pickle.dump(docsearch, f)


def pandas_df_vectorstore_loader(
    data_path: str = "indexes/samples/telemetry_sample_forecast.csv",
    page_content_column: str = "y",
):
    df = pd.read_csv(data_path)
    # jdf = df.to_dict(orient='split')
    loader = DataFrameLoader(df, page_content_column=page_content_column)
    docs = loader.load()

    # VectorStoreRetrieverMemory

    vectorstore_ts = Chroma.from_documents(
        docs, embeddings, persist_directory="croma_index"
    )
    # docs = pandas_df_vectorstore_loader(data_path=df_path,  page_content_column=data_columnn)
    vectorstore_ts.persist()

    return docs


# -- Enable the pgvector extension to work with embedding vectors
# create extension if not exists vector;

# -- Create a table to store your documents
# create table
#   documents (
#     id uuid primary key,
#     content text, -- corresponds to Document.pageContent
#     metadata jsonb, -- corresponds to Document.metadata
#     embedding vector (1536) -- 1536 works for OpenAI embeddings, change if needed
#   );

# -- Create a function to search for documents
# create function match_documents (
#   query_embedding vector (1536),
#   filter jsonb default '{}'
# ) returns table (
#   id uuid,
#   content text,
#   metadata jsonb,
#   similarity float
# ) language plpgsql as $$
# #variable_conflict use_column
# begin
#   return query
#   select
#     id,
#     content,
#     metadata,
#     1 - (documents.embedding <=> query_embedding) as similarity
#   from documents
#   where metadata @> filter
#   order by documents.embedding <=> query_embedding;
# end;
# $$;

# %%


class pgVectorStore:
    def __init__(
        self, table_name: str = "documents", query_name: str = "match_documents"
    ):
        supabase_url = os.environ.get("SUPABASE_URL")
        supabase_key = os.environ.get("SUPABASE_SERVICE_KEY")
        self.supabase: Client = create_client(supabase_url, supabase_key)
        self.embeddings = OpenAIEmbeddings()
        self.table_name = table_name
        self.query_name = query_name
        self.vector_store = self.get_vector_store()

    def get_vector_store(self) -> VectorStore:
        return SupabaseVectorStore(
            embedding=self.embeddings,
            client=self.supabase,
            table_name=self.table_name,
            query_name=self.query_name,
        )

    def read(self, query):
        matched_docs = self.vector_store.similarity_search(query)
        return matched_docs[0].page_content

    def write(
        self,
        loader: BaseLoader,
        chunk_size: int = 1000,
        chunk_overlap: int = 80,
    ):
        documents = loader.load()
        text_splitter = CharacterTextSplitter(
            chunk_size=chunk_size, chunk_overlap=chunk_overlap
        )
        docs = text_splitter.split_documents(documents)
        self.vector_store.from_documents(
            docs,
            self.embeddings,
            client=self.supabase,
            table_name=self.table_name,
            query_name=self.query_name,
            chunk_size=chunk_size,
        )


# %%

# vec = VectorStore()
# loader = firecrawl_loader('https://python.langchain.com/v0.1/docs/integrations/vectorstores/supabase/')
# vec.write(loader)


class PerUserVectorstore(RunnableSerializable):
    """A custom runnable that returns a list of documents for the given user.

    The runnable is configurable by the user, and the search results are
    filtered by the user ID.
    """

    user_id: Optional[str]
    vectorstore: VectorStore

    class Config:
        # Allow arbitrary types since VectorStore is an abstract interface
        # and not a pydantic model
        arbitrary_types_allowed = True

    def _invoke(
        self, input: str, config: Optional[RunnableConfig] = None, **kwargs: Any
    ) -> List[Document]:
        """Invoke the retriever."""
        # WARNING: Verify documentation of underlying vectorstore to make
        # sure that it actually uses filters.
        # Highly recommended to use unit-tests to verify this behavior, as
        # implementations can be different depending on the underlying vectorstore.
        # retriever = self.vectorstore.as_retriever(
        #     search_kwargs={"filter": {"owner_id": self.user_id}}
        # )
        # return retriever.invoke(input, config=config)
        matched_docs = self.vector_store.similarity_search(input)
        return matched_docs[0].page_content

    def invoke(
        self, input: str, config: Optional[RunnableConfig] = None, **kwargs
    ) -> List[Document]:
        """Add one to an integer."""
        return self._call_with_config(self._invoke, input, config, **kwargs)


async def per_req_config_modifier(config: Dict, request: Request) -> Dict:
    from arcan.api import get_current_active_user_from_request

    """Modify the config for each request."""
    user = await get_current_active_user_from_request(request)
    config["configurable"] = {}
    # Attention: Make sure that the user ID is over-ridden for each request.
    # We should not be accepting a user ID from the user in this case!
    config["configurable"]["user_id"] = user.username
    return config


def get_per_user_retriever(vectorstore: VectorStore, user_id: str = None):
    per_user_retriever = PerUserVectorstore(
        user_id=user_id,
        vectorstore=vectorstore,
    ).configurable_fields(
        # Attention: Make sure to override the user ID for each request in the
        # per_req_config_modifier. This should not be client configurable.
        user_id=ConfigurableField(
            id="user_id",
            name="User ID",
            description="The user ID to use for the retriever.",
        )
    )
    return per_user_retriever


# %%
