"""Generated from Capsem OpenAPI. Do not edit."""

from __future__ import annotations

from enum import StrEnum


class CredentialEventType(StrEnum):
    HTTP_REQUEST = 'http.request'
    HTTP_RESPONSE = 'http.response'
    MODEL_CALL = 'model.call'
    MCP_TOOL_CALL = 'mcp.tool_call'
    MCP_TOOL_LIST = 'mcp.tool_list'
    MCP_EVENT = 'mcp.event'
    DNS_QUERY = 'dns.query'
    FILE_EVENT = 'file.event'
    FILE_IMPORT = 'file.import'
    FILE_EXPORT = 'file.export'
    PROCESS_EXEC = 'process.exec'
    PROCESS_EXEC_COMPLETE = 'process.exec_complete'
    PROCESS_AUDIT = 'process.audit'
    CREDENTIAL_SUBSTITUTION = 'credential.substitution'
    SECURITY_RULE = 'security.rule'
    SECURITY_ASK = 'security.ask'
