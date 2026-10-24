"""Execute the canonical Rust checks inside the Linux qualification owner."""
from capsem_builder.gate.linuxrustchecks import main

if __name__ == '__main__':
    raise SystemExit(main())
