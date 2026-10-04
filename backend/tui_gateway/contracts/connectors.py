"""Contracts for connection-operation errors."""

from __future__ import annotations

from .base import WireEnum


class ConnectorErrorReason(WireEnum):
    invalid_params = "INVALID_PARAMS"
    not_owner = "NOT_OWNER"
    unsupported_runtime = "UNSUPPORTED_RUNTIME"
    connector_request_failed = "CONNECTOR_REQUEST_FAILED"
    unknown_target = "UNKNOWN_TARGET"
    link_still_valid = "LINK_STILL_VALID"
    reissue_refused = "REISSUE_REFUSED"
    unknown_operation = "UNKNOWN_OPERATION"
    invalid_answer = "INVALID_ANSWER"
