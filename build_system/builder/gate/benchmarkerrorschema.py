"""Error-path observations never certify disabled successful functionality."""
from typing import Literal
from uuid import UUID

from pydantic import Field, PositiveFloat, PositiveInt, model_validator

from .configschema import Strict


class ErrorEndpoint(Strict):
    classification: Literal['missing-transaction','missing-connection','unconfigured-profile','stopped-owner','missing-grant']
    status: Literal[404,503]
    code: Literal['transaction_expired','connection_unavailable','configuration_required','session_unavailable','grant_unavailable']
    reason: str = Field(min_length=20)


class ConnectionErrorConfig(Strict):
    success_coverage: Literal[False]
    samples: PositiveInt = Field(le=32)
    quick_samples: PositiveInt = Field(le=32)
    timeout_seconds: PositiveFloat = Field(allow_inf_nan=False,le=30)
    max_response_bytes: PositiveInt = Field(le=65536)
    transaction_id: str
    connection_id: str
    grant_id: str
    request_id: str
    endpoints: dict[str,ErrorEndpoint] = Field(min_length=7,max_length=7)

    @model_validator(mode='after')
    def exact_private_contracts(self):
        if self.quick_samples > self.samples:
            raise ValueError('quick error sample count exceeds full count')
        ids = (self.transaction_id,self.connection_id,self.grant_id,self.request_id)
        if len(set(ids)) != 4:
            raise ValueError('error fixture identities must be distinct')
        for value in ids:
            parsed = UUID(value)
            if str(parsed) != value or parsed.version != 4:
                raise ValueError('error fixture requires canonical valid opaque UUIDv4')
        expected = {
            'GET /connections/authorizations/{transaction_id}':(404,'transaction_expired','missing-transaction'),
            'DELETE /connections/authorizations/{transaction_id}':(404,'transaction_expired','missing-transaction'),
            'GET /connections/{connection_id}':(404,'connection_unavailable','missing-connection'),
            'DELETE /connections/{connection_id}':(404,'connection_unavailable','missing-connection'),
            'POST /connections/google/authorize':(503,'configuration_required','unconfigured-profile'),
            'POST /vms/{id}/credential-grants':(503,'session_unavailable','stopped-owner'),
            'DELETE /vms/{id}/credential-grants/{grant_id}':(404,'grant_unavailable','missing-grant'),
        }
        if set(self.endpoints) != set(expected):
            raise ValueError('error cases must retain exactly seven intentionally unavailable operations')
        if any((entry.status,entry.code,entry.classification) != expected[op] for op,entry in self.endpoints.items()):
            raise ValueError('error case cannot relabel auth/malformed/generic failures')
        return self
