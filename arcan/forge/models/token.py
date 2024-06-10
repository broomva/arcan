from sqlalchemy import Column, ForeignKey, Integer, String
from sqlalchemy.orm import relationship

from arcan.forge.database.session import Base


class Token(Base):
    __tablename__ = "token"

    id = Column(Integer, primary_key=True, index=True)
    access_token = Column(String, nullable=False)
    token_type = Column(String, nullable=False)
    user_id = Column(Integer, ForeignKey("users.id"), nullable=False)

    user = relationship("User", back_populates="token")
