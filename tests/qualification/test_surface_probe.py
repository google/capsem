"""Observed /proc command formats must preserve Chromium sandbox assertions."""

from tests.qualification.surface_probe import display_environment, process_arguments


def test_nul_delimited_arguments_keep_spaces_inside_an_argument():
    assert process_arguments(b'/opt/antigravity/antigravity\0--type=renderer\0a b\0') == [
        b'/opt/antigravity/antigravity', b'--type=renderer', b'a b',
    ]


def test_rewritten_electron_process_title_retains_type_and_sandbox_flags():
    assert process_arguments(
        b'/opt/antigravity/antigravity --type=zygote --no-zygote-sandbox\0'
    ) == [b'/opt/antigravity/antigravity', b'--type=zygote', b'--no-zygote-sandbox']
    assert b'--no-sandbox' in process_arguments(
        b'/opt/antigravity/antigravity --type=renderer --no-sandbox\0'
    )


def test_display_identity_comes_from_the_observed_xserver_arguments():
    assert display_environment([
        b'/usr/lib/xorg/Xorg-for-Xpra-100', b'-nolisten', b'tcp',
        b'-auth', b'/home/capsem/.Xauthority', b'-depth', b'24', b':100',
    ]) == {'DISPLAY': ':100', 'XAUTHORITY': '/home/capsem/.Xauthority'}
    assert display_environment([b'Xorg', b'-depth', b'24']) == {}
