"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from typing import TypeAlias

from .audit_history_details import AuditHistoryDetails
from .exec_history_details import ExecHistoryDetails

HistoryDetails: TypeAlias = ExecHistoryDetails | AuditHistoryDetails
