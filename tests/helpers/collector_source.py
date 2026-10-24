"""Source wiring must replace every finite collector before any main setup."""


def source_main(namespace,*,test_module):
    unmocked = [name for name,value in namespace.items()
        if name.startswith('collect_') and callable(value) and getattr(value,'__module__',None) != test_module]
    assert not unmocked, f'source wiring requires test-owned collectors: {unmocked}'
    return namespace['main']()
