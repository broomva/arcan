"""Health check routes."""

from fastapi import APIRouter

router = APIRouter()


@router.get("/")
async def health_check():
    """Basic health check endpoint."""
    return {"status": "healthy", "service": "arcan-api"}


@router.get("/ready")
async def readiness_check():
    """Readiness check for Kubernetes."""
    # TODO: Add checks for database, external services, etc.
    return {"status": "ready", "checks": {"api": "ok"}}


@router.get("/live")
async def liveness_check():
    """Liveness check for Kubernetes."""
    return {"status": "alive"} 