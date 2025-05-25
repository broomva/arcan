"""Workflow models using SQLModel."""

from datetime import datetime
from typing import Optional, Dict, Any
from uuid import UUID, uuid4

from sqlalchemy import Column
from sqlalchemy.dialects.postgresql import JSONB
from sqlmodel import Field, SQLModel, Relationship


class WorkflowBase(SQLModel):
    """Base workflow model with shared fields."""
    
    name: str = Field(index=True, nullable=False)
    description: str
    agent_id: UUID = Field(foreign_key="agents.id")
    graph_definition: Dict[str, Any] = Field(sa_column=Column(JSONB))
    status: str = Field(default="draft")
    version: str = Field(default="1.0.0")


class Workflow(WorkflowBase, table=True):
    """Workflow database model."""
    
    __tablename__ = "workflows"
    
    id: UUID = Field(default_factory=uuid4, primary_key=True)
    tenant_id: UUID = Field(index=True, nullable=False)
    created_at: datetime = Field(default_factory=datetime.utcnow, nullable=False)
    updated_at: datetime = Field(default_factory=datetime.utcnow, nullable=False)
    
    # Relationships
    # agent: Optional["Agent"] = Relationship(back_populates="workflows")


class WorkflowCreate(WorkflowBase):
    """Schema for creating a workflow."""
    pass


class WorkflowUpdate(SQLModel):
    """Schema for updating a workflow."""
    
    name: Optional[str] = None
    description: Optional[str] = None
    graph_definition: Optional[Dict[str, Any]] = None
    status: Optional[str] = None
    version: Optional[str] = None


class WorkflowRead(WorkflowBase):
    """Schema for reading a workflow."""
    
    id: UUID
    tenant_id: UUID
    created_at: datetime
    updated_at: datetime 