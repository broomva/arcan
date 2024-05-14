import re
from typing import Any, Dict

from fastapi import HTTPException, Request

# def fetch_api_key_from_header(config: Dict[str, Any], req: Request) -> Dict[str, Any]:
#     if "x-api-key" in req.headers:
#         config["configurable"]["openai_api_key"] = req.headers["x-api-key"]
#         if "user_id" in req.headers:
#             config["configurable"]["user_id"] = req.headers["user_id"]
#         else:
#             raise HTTPException(401, "No User ID provided")
#     else:
#         raise HTTPException(401, "No API key provided")

#     return config


def fetch_session_from_header(config: Dict[str, Any], req: Request) -> Dict[str, Any]:
    config = config.copy()
    configurable = config.get("configurable", {})

    if "arcanai_api_key" in req.headers:
        if "user_id" in req.headers:
            configurable["user_id"] = req.headers["user_id"]
            config["configurable"] = configurable
            # config["configurable"]["user_id"] = req.headers["user_id"]
            # config["configurable"] = configurable
    else:
        raise HTTPException(401, "No Arcan AI API key provided")
    return config


def _is_valid_identifier(value: str) -> bool:
    """Check if the value is a valid identifier."""
    # Use a regular expression to match the allowed characters
    valid_characters = re.compile(r"^[a-zA-Z0-9-_]+$")
    return bool(valid_characters.match(value))
