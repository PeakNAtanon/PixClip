#!/usr/bin/env python3
import sys

from media_toolkit import main


if __name__ == '__main__':
    raise SystemExit(main(['--install-tools', *sys.argv[1:]]))
