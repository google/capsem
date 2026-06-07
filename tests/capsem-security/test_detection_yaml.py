from pathlib import Path

from sigma.collection import SigmaCollection


ROOT = Path(__file__).resolve().parents[2]
DETECTION_FIXTURE = ROOT / "sprints/security-event-rule-spine/fixtures/detection.yaml"


def test_detection_yaml_parses_with_pysigma() -> None:
    collection = SigmaCollection.from_yaml(DETECTION_FIXTURE.read_text())

    assert len(collection.rules) == 1
    rule = collection.rules[0]
    assert rule.title == "OpenAI Traffic To Unexpected Endpoint"
    assert rule.logsource.product == "capsem"
    assert rule.logsource.service == "security_event"
    assert rule.level.name.lower() == "high"
    assert rule.custom_attributes["capsem"]["action"] == "block"
