from __future__ import annotations

from typing import cast

import pytest
from capsem_builder.policy.dockerpolicy import (
    BuildNetwork,
    ContainerNetwork,
    require_build_network,
    require_container_network,
)


def test_docker_network_boundaries_reject_the_other_enum_family() -> None:
    assert require_build_network(BuildNetwork.NONE) == "none"
    assert require_container_network(ContainerNetwork.NONE) == "none"
    with pytest.raises(TypeError, match="BuildNetwork"):
        require_build_network(cast(BuildNetwork, ContainerNetwork.NONE))
    with pytest.raises(TypeError, match="ContainerNetwork"):
        require_container_network(cast(ContainerNetwork, BuildNetwork.NONE))
