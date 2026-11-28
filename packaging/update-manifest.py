#!/usr/bin/env python3
"""Write and sign the auto-update manifest for a release.

    update-manifest.py sign DIST_DIR VERSION    (key from env UPDATE_SIGNING_KEY)
    update-manifest.py keygen KEY_FILE          (writes a new private key, prints the public key)

The manifest, DIST_DIR/ledge-grab-update.txt, is what installed copies of Ledge
Grab download to learn about a new release:

    ledge-grab-update 1
    version 0.1.3
    file LedgeGrab-0.1.3-win-x64.exe 4194304 <sha256 hex>
    file LedgeGrab-Setup-0.1.3-x64.exe 5242880 <sha256 hex>
    sig <base64 ed25519 signature of every byte above this line>

The app verifies the signature with the public key compiled into it (src/update.rs)
and refuses anything else. UPDATE_SIGNING_KEY is the base64 32-byte private seed.
Needs the `cryptography` package.
"""
import base64, hashlib, os, sys
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization

NAME = "ledge-grab-update.txt"


def raw_public(key):
    return key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


def keygen(path):
    if os.path.exists(path):
        sys.exit(f"{path} exists; not overwriting a key")
    key = Ed25519PrivateKey.generate()
    seed = key.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw, serialization.NoEncryption())
    with open(path, "w") as f:
        f.write(base64.b64encode(seed).decode() + "\n")
    print("public key (hex):", raw_public(key).hex())


def sign(dist, version):
    seed = os.environ.get("UPDATE_SIGNING_KEY", "").strip()
    if not seed:
        sys.exit("UPDATE_SIGNING_KEY is not set; refusing to publish a release installed copies can't verify")
    key = Ed25519PrivateKey.from_private_bytes(base64.b64decode(seed))
    names = [f"LedgeGrab-{version}-win-x64.exe", f"LedgeGrab-Setup-{version}-x64.exe"]
    body = f"ledge-grab-update 1\nversion {version}\n"
    for n in names:
        p = os.path.join(dist, n)
        data = open(p, "rb").read()
        body += f"file {n} {len(data)} {hashlib.sha256(data).hexdigest()}\n"
    sig = base64.b64encode(key.sign(body.encode())).decode()
    with open(os.path.join(dist, NAME), "w", newline="\n") as f:
        f.write(body + f"sig {sig}\n")
    print(f"wrote {NAME} for {version}; public key {raw_public(key).hex()}")


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "sign":
        sign(sys.argv[2], sys.argv[3])
    elif len(sys.argv) == 3 and sys.argv[1] == "keygen":
        keygen(sys.argv[2])
    else:
        sys.exit(__doc__)
