"""Local-only capture storage. Hidden previews are a metadata decision, not a delete."""
from datetime import datetime
from math import ceil, floor
from pathlib import Path
from uuid import uuid4
import json
import os
import re
import tempfile

def data_directory():
    override = os.environ.get("LEDGE_GRAB_DIR")
    if override:
        return Path(override).expanduser().resolve()
    pictures = Path.home() / "Pictures"
    return pictures / "Ledge Grab"

class Storage:
    def __init__(self, root):
        self.root = Path(root)
        self.root.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.state = self.root / ".hidden-previews.json"
        try:
            document = json.loads(self.state.read_text())
            self.hidden = set(x for x in document if isinstance(x, str)) if isinstance(document,list) else set()
        except (OSError, ValueError):
            self.hidden = set()

    def _owned(self, path):
        path = Path(path)
        if path.is_symlink() or path.parent.resolve() != self.root.resolve():
            raise ValueError("Not a local Ledge Grab capture.")
        if not path.name.startswith("LedgeGrab-") or path.suffix != ".png":
            raise ValueError("Not a Ledge Grab capture.")
        return path

    def save(self, write_png):
        now = datetime.now().strftime("%Y%m%d-%H%M%S")
        final = self.root / f"LedgeGrab-{now}-{uuid4().hex[:10]}.png"
        fd, tmp = tempfile.mkstemp(prefix=".capture-", dir=self.root)
        os.close(fd)
        try:
            write_png(tmp)
            with open(tmp, "rb") as f:
                if f.read(8) != b"\x89PNG\r\n\x1a\n":
                    raise ValueError("Capture did not produce a PNG.")
            os.replace(tmp, final)
        finally:
            Path(tmp).unlink(missing_ok=True)
        return final

    def recent(self, limit=8):
        pattern = re.compile(r"LedgeGrab-\d{8}-\d{6}-[0-9a-f]{10}\.png")
        rows=[]
        for path in self.root.iterdir():
            if not pattern.fullmatch(path.name) or path.name in self.hidden or path.is_symlink():
                continue
            try:
                if path.is_file(): rows.append((path.stat().st_mtime_ns,path))
            except FileNotFoundError:
                pass
        return [path for _mtime,path in sorted(rows,reverse=True)[:limit]]

    def hide(self, path):
        path = self._owned(path)
        new = self.hidden | {path.name}
        fd, tmp = tempfile.mkstemp(prefix=".hidden-", dir=self.root)
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                json.dump(sorted(new),f)
            os.replace(tmp,self.state)
        finally:
            Path(tmp).unlink(missing_ok=True)
        self.hidden = new

def crop_bounds(start, end, logical, physical):
    """Map a normalized drag rectangle to bounded image-pixel coordinates."""
    if min(*logical, *physical) <= 0:
        raise ValueError("Invalid screen dimensions.")
    xy = []
    for a,b,l,p in zip(start,end,logical,physical):
        lo,hi = sorted((a,b))
        lo,hi = (max(0,min(l,v)) for v in (lo,hi))
        left = max(0,min(p, floor(lo*p/l)))
        right = max(0,min(p, ceil(hi*p/l)))
        xy.append((left,right))
    (x,r),(y,b) = xy
    return x,y,r-x,b-y
