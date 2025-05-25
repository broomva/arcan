"""Agent service for database operations."""

from typing import Optional
from uuid import UUID

from sqlalchemy import select, func
from sqlalchemy.ext.asyncio import AsyncSession

from src.db.models.agent import Agent, AgentCreate, AgentUpdate


class AgentService:
    """Service class for agent database operations."""
    
    def __init__(self, session: AsyncSession):
        self.session = session
    
    async def create(self, agent: AgentCreate, tenant_id: UUID) -> Agent:
        """Create a new agent."""
        db_agent = Agent(
            **agent.model_dump(),
            tenant_id=tenant_id,
        )
        self.session.add(db_agent)
        await self.session.commit()
        await self.session.refresh(db_agent)
        return db_agent
    
    async def get(self, agent_id: UUID, tenant_id: UUID) -> Optional[Agent]:
        """Get an agent by ID."""
        statement = select(Agent).where(
            Agent.id == agent_id,
            Agent.tenant_id == tenant_id,
        )
        result = await self.session.execute(statement)
        return result.scalar_one_or_none()
    
    async def list(
        self,
        tenant_id: UUID,
        skip: int = 0,
        limit: int = 20,
        name: Optional[str] = None,
        status: Optional[str] = None,
    ) -> tuple[list[Agent], int]:
        """List agents with pagination and filtering."""
        # Build query
        query = select(Agent).where(Agent.tenant_id == tenant_id)
        
        if name:
            query = query.where(Agent.name.ilike(f"%{name}%"))
        if status:
            query = query.where(Agent.status == status)
        
        # Get total count
        count_query = select(func.count()).select_from(Agent).where(Agent.tenant_id == tenant_id)
        if name:
            count_query = count_query.where(Agent.name.ilike(f"%{name}%"))
        if status:
            count_query = count_query.where(Agent.status == status)
        
        total_result = await self.session.execute(count_query)
        total = total_result.scalar_one()
        
        # Get paginated results
        query = query.offset(skip).limit(limit)
        result = await self.session.execute(query)
        agents = result.scalars().all()
        
        return agents, total
    
    async def update(self, agent_id: UUID, tenant_id: UUID, agent_update: AgentUpdate) -> Optional[Agent]:
        """Update an agent."""
        agent = await self.get(agent_id, tenant_id)
        if not agent:
            return None
        
        update_data = agent_update.model_dump(exclude_unset=True)
        for field, value in update_data.items():
            setattr(agent, field, value)
        
        await self.session.commit()
        await self.session.refresh(agent)
        return agent
    
    async def delete(self, agent_id: UUID, tenant_id: UUID) -> bool:
        """Delete an agent."""
        agent = await self.get(agent_id, tenant_id)
        if not agent:
            return False
        
        await self.session.delete(agent)
        await self.session.commit()
        return True 