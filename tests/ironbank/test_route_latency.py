"""Compatibility pointer for the route-health performance probes.

The release timing rail owns ``test_route_health.py`` directly. This module
deliberately defines no tests: wrappers around those tests made pytest execute
the same stateful measurement twice and allowed one CPU-accounting tick to
produce contradictory verdicts in a single qualification.
"""
