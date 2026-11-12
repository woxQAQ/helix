"""Lock selected grammar sources; ordinary Nix evaluation never fetches them."""

import argparse
import concurrent.futures
import json
import re
import subprocess
import tempfile
import tomllib
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument(
    "--check",
    action="store_true",
    help="Check the lock file without fetching or writing",
)
parser.add_argument(
    "--jobs", type=int, default=4, help="Concurrent source fetches (default: 4)"
)
args = parser.parse_args()
if args.jobs < 1:
    parser.error("--jobs must be positive")

root = Path(__file__).resolve().parent.parent
config = tomllib.loads((root / "languages.toml").read_text())
selection = config.get("use-grammars", {})
grammars = {
    grammar["name"]: grammar["source"]
    for grammar in config["grammar"]
    if "git" in grammar["source"]
    and "rev" in grammar["source"]
    and ("only" not in selection or grammar["name"] in selection["only"])
    and ("only" in selection or grammar["name"] not in selection.get("except", []))
}
lock_path = root / "nix/grammar-sources.json"
locked = json.loads(lock_path.read_text()) if lock_path.exists() else {}
pending = {
    name: source
    for name, source in grammars.items()
    if locked.get(name, {}).get("source") != source
    or not re.fullmatch(
        r"sha256-[A-Za-z0-9+/]{43}=", locked.get(name, {}).get("hash", "")
    )
}
removed = locked.keys() - grammars.keys()
if args.check:
    if pending or removed:
        raise SystemExit(
            "Grammar source lock is stale. Run python3 nix/update-grammars.py\n"
            f"Missing/changed: {', '.join(sorted(pending)) or '(none)'}\n"
            f"Removed/excluded: {', '.join(sorted(removed)) or '(none)'}"
        )
    print(f"All {len(grammars)} grammar sources are locked.")
    raise SystemExit(0)

# Multiple grammars can share a repository and revision with different subpaths.
fetches = {}
for name, source in pending.items():
    identity = (source["git"], source["rev"], source.get("ref", "HEAD"))
    fetches.setdefault(identity, []).append(name)


def prefetch(identity):
    url, rev, ref = identity
    github = re.fullmatch(r"https://github\.com/([^/]+)/([^/]+)/?", url)
    if github:
        tree = {"type": "github", "owner": github[1], "repo": github[2], "rev": rev}
    else:
        tree = {"type": "git", "url": url, "rev": rev, "ref": ref, "shallow": True}
    # Keep source values in JSON so neither shell nor Nix interpolation can interpret them.
    with tempfile.TemporaryDirectory(prefix="helix-grammar-") as directory:
        request = Path(directory) / "source.json"
        request.write_text(json.dumps(tree))
        result = subprocess.run(
            [
                "nix",
                "eval",
                "--json",
                "--impure",
                "--expr",
                f"let source = builtins.fetchTree (builtins.fromJSON (builtins.readFile {request})); in {{ inherit (source) narHash; }}",
            ],
            text=True,
            capture_output=True,
            check=False,
        )
    if result.returncode:
        raise RuntimeError(f"{url}@{rev}:\n{result.stderr}")
    return json.loads(result.stdout)["narHash"]


updated = {name: locked[name] for name in grammars if name not in pending}
failures = []
with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as executor:
    tasks = {
        executor.submit(prefetch, identity): names
        for identity, names in fetches.items()
    }
    for index, task in enumerate(concurrent.futures.as_completed(tasks), 1):
        names = tasks[task]
        try:
            hash_value = task.result()
            for name in names:
                updated[name] = {"source": grammars[name], "hash": hash_value}
        except (RuntimeError, OSError, ValueError) as error:
            failures.append(f"{', '.join(names)}: {error}")
        if index % 20 == 0 or index == len(tasks):
            print(f"Fetched {index}/{len(tasks)} distinct sources", flush=True)
if failures:
    raise SystemExit("Lock file was not changed:\n" + "\n".join(failures))

# Replace only after all sources succeeded, preserving the previous lock on failure.
with tempfile.NamedTemporaryFile(
    mode="w", dir=lock_path.parent, delete=False
) as temporary:
    temporary.write(json.dumps(updated, indent=2, sort_keys=True) + "\n")
    temporary_path = Path(temporary.name)
temporary_path.replace(lock_path)
print(f"Locked {len(updated)} grammar sources in {lock_path.relative_to(root)}")
