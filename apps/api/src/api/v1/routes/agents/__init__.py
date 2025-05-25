"""Agent-related endpoints."""

from typing import Optional
from uuid import UUID

from fastapi import APIRouter, Depends, HTTPException, Query
from sqlalchemy.ext.asyncio import AsyncSession

from arcan import __version__ as arcan_version
from src.db.database import get_session
from src.db.models.agent import AgentCreate, AgentList, AgentRead, AgentUpdate
from src.db.services.agent import AgentService

router = APIRouter()

# TODO: This should come from authentication/JWT
# For now, using a hardcoded tenant ID
TEMP_TENANT_ID = UUID("00000000-0000-0000-0000-000000000001")


@router.get("/", response_model=AgentList)
async def list_agents(
    page: int = Query(1, ge=1),
    page_size: int = Query(20, ge=1, le=100),
    name: Optional[str] = None,
    status: Optional[str] = None,
    session: AsyncSession = Depends(get_session),
) -> AgentList:
    """List all agents with pagination."""
    service = AgentService(session)
    skip = (page - 1) * page_size
    
    agents, total = await service.list(
        tenant_id=TEMP_TENANT_ID,
        skip=skip,
        limit=page_size,
        name=name,
        status=status,
    )
    
    return AgentList(
        agents=[AgentRead.model_validate(agent) for agent in agents],
        total=total,
        page=page,
        page_size=page_size,
    )


@router.post("/", response_model=AgentRead)
async def create_agent(
    agent: AgentCreate,
    session: AsyncSession = Depends(get_session),
) -> AgentRead:
    """Create a new agent."""
    service = AgentService(session)
    db_agent = await service.create(agent, tenant_id=TEMP_TENANT_ID)
    return AgentRead.model_validate(db_agent)


@router.get("/{agent_id}", response_model=AgentRead)
async def get_agent(
    agent_id: UUID,
    session: AsyncSession = Depends(get_session),
) -> AgentRead:
    """Get a specific agent by ID."""
    service = AgentService(session)
    agent = await service.get(agent_id, tenant_id=TEMP_TENANT_ID)
    
    if not agent:
        raise HTTPException(status_code=404, detail="Agent not found")
    
    return AgentRead.model_validate(agent)


@router.patch("/{agent_id}", response_model=AgentRead)
async def update_agent(
    agent_id: UUID,
    agent_update: AgentUpdate,
    session: AsyncSession = Depends(get_session),
) -> AgentRead:
    """Update an agent."""
    service = AgentService(session)
    agent = await service.update(agent_id, TEMP_TENANT_ID, agent_update)
    
    if not agent:
        raise HTTPException(status_code=404, detail="Agent not found")
    
    return AgentRead.model_validate(agent)


@router.delete("/{agent_id}")
async def delete_agent(
    agent_id: UUID,
    session: AsyncSession = Depends(get_session),
) -> dict:
    """Delete an agent."""
    service = AgentService(session)
    deleted = await service.delete(agent_id, TEMP_TENANT_ID)
    
    if not deleted:
        raise HTTPException(status_code=404, detail="Agent not found")
    
    return {"message": "Agent deleted successfully"}


@router.get("/version", response_model=dict)
async def get_arcan_version() -> dict:
    """Get the version of the Arcan library being used."""
    return {"arcan_version": arcan_version, "api_version": "0.1.0"} 