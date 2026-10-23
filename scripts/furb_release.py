"""Name the five crates of this fork that furb runs as furb-sandbox, for a release of furb on crates.io.

Run it from the root of the repository, with the version of the release, on a commit of the line of furb:
`python3 scripts/furb_release.py 0.2.0`. It changes the five Cargo.toml, the workspace, and the README and LICENSE of
each crate, and changes nothing else, so a release is the line of furb plus one commit that this script makes. Each
crate keeps the name of its library, and the workspace names each by its old key, so no line of code changes. The line
of furb keeps the names of monty, so that it merges with pydantic/monty and runs its own CI.
"""

import pathlib
import re
import sys

NAMES = {
    'monty': 'furb-sandbox',
    'monty-types': 'furb-sandbox-types',
    'monty-macros': 'furb-sandbox-macros',
    'monty-type-checking': 'furb-sandbox-type-checking',
    'monty-typeshed': 'furb-sandbox-typeshed',
}


def main(version: str) -> None:
    root = pathlib.Path()
    workspace = root / 'Cargo.toml'
    text = workspace.read_text()
    for old, new in NAMES.items():
        text, n = re.subn(
            rf'^{old} = {{ path = "crates/{old}", version = "[^"]*" }}$',
            f'{old} = {{ path = "crates/{old}", version = "{version}", package = "{new}" }}',
            text,
            flags=re.MULTILINE,
        )
        assert n == 1, old
    workspace.write_text(text)
    notice = (root / 'LICENSE').read_text()
    for old, new in NAMES.items():
        crate = root / 'crates' / old
        manifest = crate / 'Cargo.toml'
        text = manifest.read_text()
        text, n = re.subn(
            rf'^name = "{old}"\n',
            f'name = "{new}"\nrepository = "https://github.com/uael/monty"\n',
            text,
            count=1,
            flags=re.MULTILINE,
        )
        assert n == 1, old
        text, n = re.subn(
            r'^version = \{ workspace = true \}\n', f'version = "{version}"\n', text, count=1, flags=re.MULTILINE
        )
        assert n == 1, old
        text, n = re.subn(
            r'^description = .*\n',
            f'description = "A fork of pydantic/monty for furb: the crate {old} at the revision that furb runs."\n',
            text,
            count=1,
            flags=re.MULTILINE,
        )
        assert n == 1, old
        text = re.sub(r'^(repository|homepage|documentation) = \{ workspace = true \}\n', '', text, flags=re.MULTILINE)
        if old == 'monty-macros':
            text = text.replace('[lib]\nproc-macro = true\n', '[lib]\nname = "monty_macros"\nproc-macro = true\n', 1)
        manifest.write_text(text)
        (crate / 'LICENSE').write_text(notice)
        readme = crate / 'README.md'
        readme.write_text(
            f'> This is `{new}`, a fork of [`{old}`](https://github.com/pydantic/monty) by Pydantic, for\n'
            '> [furb](https://github.com/uael/furb). It holds the changes that furb needs and that no release of monty holds\n'
            '> yet. Its source is https://github.com/uael/monty, under the MIT license of Pydantic, which `LICENSE` holds.\n'
            f'> Depend on `{old}` unless you need furb.\n\n' + readme.read_text()
        )


if __name__ == '__main__':
    main(sys.argv[1])
