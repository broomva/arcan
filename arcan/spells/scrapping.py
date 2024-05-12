# %%
import json
import os
import time

import html2text
import requests
from bs4 import BeautifulSoup
from dotenv import load_dotenv
from firecrawl import FirecrawlApp
from langchain.agents import Tool
from langchain_community.tools import WikipediaQueryRun
from langchain_community.tools.tavily_search import TavilySearchResults
from langchain_community.utilities import WikipediaAPIWrapper
from pydantic import AnyHttpUrl, FilePath
from selenium import webdriver
from selenium.webdriver.chrome.options import Options

brwoserless_api_key = os.getenv("BROWSERLESS_API_KEY")


def scrape_website(url: str):
    # scrape website, and also will summarize the content based on objective if the content is too large
    # objective is the original objective & task that user give to the agent, url is the url of the website to be scraped

    print("Scraping website...")
    # Define the headers for the request
    headers = {
        "Cache-Control": "no-cache",
        "Content-Type": "application/json",
    }

    # Define the data to be sent in the request
    data = {"url": url}

    # Convert Python object to JSON string
    data_json = json.dumps(data)

    # Send the POST request
    response = requests.post(
        f"https://chrome.browserless.io/content?token={brwoserless_api_key}",
        headers=headers,
        data=data_json,
        timeout=60,
    )

    # Check the response status code
    if response.status_code == 200:
        soup = BeautifulSoup(response.content, "html.parser")
        text = soup.get_text()
        if len(text) < 100:
            raise Exception("Content too short")
        return text
    else:
        raise Exception(f"HTTP request failed with status code {response.status_code}")


def scrape_website_selenium(url):
    try:
        # Configure Selenium with a headless browser
        options = Options()
        options.headless = True
        driver = webdriver.Chrome(options=options)

        # Access the webpage
        driver.get(url)

        # Wait for JavaScript to render. Adjust time as needed.
        time.sleep(5)  # Time in seconds

        # Extract the page source
        page_source = driver.page_source

        # Close the browser
        driver.quit()

        # Convert HTML to Markdown
        converter = html2text.HTML2Text()
        markdown = converter.handle(page_source)
        if len(markdown) < 100:
            raise Exception("Content too short")

        return markdown
    except Exception as e:
        print(f"Error scraping website: {e}")
        raise e


import os
import re
from pathlib import Path

import httpx
from bs4 import BeautifulSoup


def scrape_url(url) -> str:
    # fetch article; simulate desktop browser
    headers = {
        "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_11_2) AppleWebKit/601.3.9 (KHTML, like Gecko) Version/9.0.2 Safari/601.3.9"
    }
    response = httpx.get(url, headers=headers)
    soup = BeautifulSoup(response.text, "lxml")

    for tag in soup.find_all():
        if tag.string:
            stripped_string = tag.string.strip()
            tag.string.replace_with(stripped_string)

    text = soup.get_text()
    clean_text = text.replace("\n\n", "\n")

    return clean_text.replace("\t", "")


def url_text_scrapper(url: str):
    domain_regex = r"(?:https?:\/\/)?(?:[^@\n]+@)?(?:www\.)?([^:\/\n\.]+)"

    match = re.search(domain_regex, url)

    if match:
        domain = match.group(1)
        clean_domain = re.sub(r"[^a-zA-Z0-9]+", "", domain)

    # Support caching speech text on disk.
    file_path = Path(f"scrappings/{clean_domain}.txt")
    print(file_path)

    if file_path.exists():
        scrapped_text = file_path.read_text()
    else:
        print("Scrapping from url")
        scrapped_text = scrape_url(url)
        os.makedirs(file_path.parent, exist_ok=True)
        file_path.write_text(scrapped_text)

    return scrapped_text, clean_domain


def firecrawl_loader(url: str, mode: str = "scrape"):
    from langchain_community.document_loaders import FireCrawlLoader

    loader = FireCrawlLoader(
        api_key=os.environ.get("FIRECRAWL_API_KEY"),
        url=url,
        mode=mode,  # scrape: Scrape single url and return the markdown.
        # crawl: Crawl the url and all accessible sub pages and return the markdown for each one.
    )
    return loader


def firecrawl_scrape(url):
    """
    The function `firecrawl_scrape` takes a URL as input and uses the FirecrawlApp class to scrape the
    content of the webpage at that URL.

    :param url: The `url` parameter in the `firecrawl_scrape` function is a string that represents the
    URL of the webpage that you want to scrape using the FirecrawlApp
    :return: The `firecrawl_scrape` function is returning the result of calling the `scrape_url` method
    of a `FirecrawlApp` instance with the provided `url` as an argument. It is a markdown string of the
    scraped content of the webpage at the provided URL.
    """
    return FirecrawlApp().scrape_url(
        url,
        {
            "extractorOptions": {
                "mode": "llm-extraction",
                "extractionPrompt": "Extract the key elements, segment by NER, and summarize the content. Make sure the returned content is at most 16385 tokens",
            },
            "pageOptions": {"onlyMainContent": True},
        },
    )
    return FirecrawlApp().scrape_url(
        url,
        {
            "extractorOptions": {
                "mode": "llm-extraction",
                "extractionPrompt": "Extract the key elements, segment by NER, and summarize the content. Make sure the returned content is at most 16385 tokens",
            },
            "pageOptions": {"onlyMainContent": True},
        },
    )


# from pydantic import AnyHttpUrl, FilePath

# def scrapegraph_scrape(url: AnyHttpUrl, prompt: str):
#     from scrapegraphai.graphs import SmartScraperGraph


#     graph_config = {
#         "llm": {
#             "model": "ollama/mistral",
#             "temperature": 0,
#             "format": "json",  # Ollama needs the format to be specified explicitly
#             "base_url": "http://localhost:11434",  # set Ollama URL
#         },
#         "embeddings": {
#             "model": "ollama/nomic-embed-text",
#             "base_url": "http://localhost:11434",  # set Ollama URL
#         },
#         "verbose": True,
#     }

#     smart_scraper_graph = SmartScraperGraph(
#         prompt=prompt,
#         # also accepts a string with the already downloaded HTML code
#         source=url.__str__(),
#         config=graph_config,
#         prompt=prompt,
#         # also accepts a string with the already downloaded HTML code
#         source=url.__str__(),
#         config=graph_config,
#     )

#     result = smart_scraper_graph.run()
#     print(result)


async def llama_parse_scrape(pdf_path: FilePath):
    import nest_asyncio

    nest_asyncio.apply()

    from llama_parse import LlamaParse

    parser = LlamaParse(
        api_key=os.environ.get("LLAMA_CLOUD_API_KEY"),
        result_type="markdown",  # "markdown" and "text" are available
        num_workers=4,  # if multiple files passed, split in `num_workers` API calls
        verbose=True,
        language="en",  # Optionally you can define a language, default=en
    )

    # async
    documents = await parser.aload_data(pdf_path)
    return documents
