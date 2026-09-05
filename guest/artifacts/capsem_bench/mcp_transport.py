"""One owned FastMCP transport import for guest benchmark collectors."""

from fastmcp import Client
from fastmcp.client.transports import StdioTransport

__all__ = ["Client", "StdioTransport"]
