"""Agent models using SQLModel."""

from datetime import datetime
from typing import Optional, Dict, Any
from uuid import UUID, uuid4

from sqlalchemy import Column
from sqlalchemy.dialects.postgresql import JSONB
from sqlmodel import Field, SQLModel


class AgentBase(SQLModel):
    """Base agent model with shared fields."""
    
    name: str = Field(index=True, nullable=False)
    description: str
    type: str = Field(default="basic")
    status: str = Field(default="active")
    agent_metadata: Optional[Dict[str, Any]] = Field(default=None, sa_column=Column(JSONB))


class Agent(AgentBase, table=True):
    """Agent database model."""
    
    __tablename__ = "agents"
    
    id: UUID = Field(default_factory=uuid4, primary_key=True)
    tenant_id: UUID = Field(index=True, nullable=False)  # For multi-tenancy
    created_at: datetime = Field(default_factory=datetime.utcnow, nullable=False)
    updated_at: datetime = Field(default_factory=datetime.utcnow, nullable=False)
    
    # Relationships will be added here (e.g., workflows, tools, etc.)


class AgentCreate(AgentBase):
    """Schema for creating an agent."""
    pass


class AgentUpdate(SQLModel):
    """Schema for updating an agent."""
    
    name: Optional[str] = None
    description: Optional[str] = None
    type: Optional[str] = None
    status: Optional[str] = None
    agent_metadata: Optional[Dict[str, Any]] = None


class AgentRead(AgentBase):
    """Schema for reading an agent."""
    
    id: UUID
    tenant_id: UUID
    created_at: datetime
    updated_at: datetime


class AgentList(SQLModel):
    """Schema for listing agents with pagination."""
    
    agents: list[AgentRead]
    total: int
    page: int
    page_size: int 