"""BacklotHub errors: every error has a stable English code; the UI translates it (BacklotHub #16)."""

from __future__ import annotations


class BacklotError(Exception):
    """Base of all BacklotHub errors. ``code`` is the stable machine-readable code."""

    code = "BACKLOT_ERROR"

    def __init__(self, detail: str = "") -> None:
        super().__init__(f"{self.code}: {detail}" if detail else self.code)
        self.detail = detail


class InvalidInput(BacklotError):
    code = "INPUT_INVALID"


class ProductionCodeTaken(BacklotError):
    code = "PRODUCTION_CODE_TAKEN"


class ProductionNotFound(BacklotError):
    code = "PRODUCTION_NOT_FOUND"


class StatusNotInList(BacklotError):
    code = "STATUS_NOT_IN_LIST"


class TypeUnknown(BacklotError):
    code = "PRODUCTION_TYPE_UNKNOWN"


class CurrencyUnknown(BacklotError):
    code = "CURRENCY_UNKNOWN"


class CountryUnknown(BacklotError):
    code = "COUNTRY_UNKNOWN"


class FpsInvalid(BacklotError):
    code = "FPS_INVALID"


class ConflictStaleVersion(BacklotError):
    code = "CONFLICT_STALE_VERSION"


class PermissionDenied(BacklotError):
    code = "PERMISSION_DENIED"


class DatabaseUnavailable(BacklotError):
    code = "DB_UNAVAILABLE"


class MilestoneNotFound(BacklotError):
    code = "MILESTONE_NOT_FOUND"


class MilestoneDone(BacklotError):
    code = "MILESTONE_DONE"


class OrganizationNotFound(BacklotError):
    code = "ORGANIZATION_NOT_FOUND"


class OrganizationInUse(BacklotError):
    code = "ORGANIZATION_IN_USE"


class OrganizationRetired(BacklotError):
    code = "ORGANIZATION_RETIRED"


class OrganizationAlreadyLinked(BacklotError):
    code = "ORGANIZATION_ALREADY_LINKED"


class OrganizationNotLinked(BacklotError):
    code = "ORGANIZATION_NOT_LINKED"


class ConfigCodeTaken(BacklotError):
    code = "CONFIG_CODE_TAKEN"


class StatusListNotFound(BacklotError):
    code = "STATUS_LIST_NOT_FOUND"


class StatusNotFound(BacklotError):
    code = "STATUS_NOT_FOUND"


class TypeInUse(BacklotError):
    code = "PRODUCTION_TYPE_IN_USE"


class StructureRuleViolation(BacklotError):
    code = "STRUCTURE_RULE_VIOLATION"


class StructureCycle(BacklotError):
    code = "STRUCTURE_CYCLE"


class StructureCrossProduction(BacklotError):
    code = "STRUCTURE_CROSS_PRODUCTION"


class StructureAssetHasParent(BacklotError):
    code = "STRUCTURE_ASSET_HAS_PARENT"


class EntityCodeTaken(BacklotError):
    code = "ENTITY_CODE_TAKEN"


class EntityInUse(BacklotError):
    code = "ENTITY_IN_USE"


class EntityNotFound(BacklotError):
    code = "ENTITY_NOT_FOUND"


class EntityTypeUnknown(BacklotError):
    code = "ENTITY_TYPE_UNKNOWN"


class TimingInvalid(BacklotError):
    code = "TIMING_INVALID"


class LinkDuplicate(BacklotError):
    code = "LINK_DUPLICATE"


class LinkNotFound(BacklotError):
    code = "LINK_NOT_FOUND"


class LinkInvalid(BacklotError):
    code = "LINK_INVALID"
