"""Finite retention for the existing native image artifact action sequence."""

from typing import Literal

from pydantic import Field, PositiveInt, model_validator

from .configschema import Strict


class NativeTransferConfig(Strict):
    stage: Literal["ironbank"]
    prefix: Literal["native-transfer"]
    hold_seconds: PositiveInt = Field(le=1800)
    ready_seconds: PositiveInt = Field(le=300)
    release_seconds: PositiveInt = Field(le=120)
    verify_seconds: PositiveInt = Field(le=300)
    outer_seconds: PositiveInt = Field(le=2400)
    upload_minutes: PositiveInt = Field(le=10)
    download_minutes: PositiveInt = Field(le=10)
    poll_seconds: float = Field(gt=0, le=1, allow_inf_nan=False)

    @model_validator(mode="after")
    def consumer_fits_owner(self):
        consumer = (
            60 * (self.upload_minutes + self.download_minutes)
            + self.verify_seconds
            + self.release_seconds
        )
        if (
            self.hold_seconds <= consumer
            or self.outer_seconds < self.ready_seconds + self.hold_seconds + self.release_seconds
        ):
            raise ValueError(
                "native upload/download/verify/release must fit the finite retained owner"
            )
        return self
