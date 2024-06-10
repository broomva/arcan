from fastapi import APIRouter, Depends

from arcan.forge.schemas.user import User
from arcan.forge.service.user import UserService

router = APIRouter()

@router.get("/me", response_model=User)
async def read_users_me(current_user: User = Depends(UserService.get_current_active_user)):
    return current_user
