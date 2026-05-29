class CrustDBError(Exception):
    """Base exception for CrustDB errors."""


class ValidationError(CrustDBError):
    """Raised when row data does not match a model schema."""


class UniqueConstraintError(CrustDBError):
    """Raised when an insert violates a primary key or unique constraint."""


class NotFoundError(CrustDBError):
    """Reserved for APIs that should raise when a row is missing."""
