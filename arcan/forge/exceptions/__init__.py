class ArcanApiError(Exception):
    """base exception class"""

    def __init__(self, message: str = "Service is unavailable", name: str = "ArcanApi"):
        self.message = message
        self.name = name
        super().__init__(self.message, self.name)


class ServiceError(ArcanApiError):
    """failures in external services or APIs, like a database or a third-party service"""

    pass


class EntityDoesNotExistError(ArcanApiError):
    """database returns nothing"""

    pass


class EntityAlreadyExistsError(ArcanApiError):
    """conflict detected, like trying to create a resource that already exists"""

    pass


class InvalidOperationError(ArcanApiError):
    """invalid operations like trying to delete a non-existing entity, etc."""

    pass


class AuthenticationFailed(ArcanApiError):
    """invalid authentication credentials"""

    pass


class InvalidTokenError(ArcanApiError):
    """invalid token"""

    pass
