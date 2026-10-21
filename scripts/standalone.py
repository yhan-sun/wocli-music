"""Entry point shared by the portable CLI and its detached playback service."""

import sys

from wocli_music.runtime import configure_certificates

configure_certificates()

if sys.argv[1:] == ["--wocli-service"]:
    from wocli_music.service import main
else:
    from wocli_music.cli import main

main()
