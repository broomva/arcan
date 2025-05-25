"""API v1 router aggregation."""

from fastapi import APIRouter

from src.api.v1.routes import agents, health, workflows

api_router = APIRouter()

# Include all v1 endpoints
api_router.include_router(health.router, prefix="/health", tags=["health"])
api_router.include_router(agents.router, prefix="/agents", tags=["agents"])
api_router.include_router(workflows.router, prefix="/workflows", tags=["workflows"]) 