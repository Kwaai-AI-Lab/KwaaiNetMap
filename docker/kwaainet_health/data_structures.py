from dataclasses import dataclass, field
from typing import Optional
from urllib.parse import urlparse

import petals


@dataclass
class ModelInfo(petals.data_structures.ModelInfo):
    dht_prefix: Optional[str] = None
    official: bool = True
    limited: bool = False

    @classmethod
    def from_dict(cls, source: dict):
        return cls(**source)

    @property
    def name(self) -> str:
        return urlparse(self.repository).path.lstrip("/")

    @property
    def short_name(self) -> str:
        return self.name.split("/")[-1]
