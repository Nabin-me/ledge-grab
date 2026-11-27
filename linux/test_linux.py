import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from gi.repository import Gio, GLib
import portal
from storage import Storage, crop_bounds

PNG = b"\x89PNG\r\n\x1a\n"

class CaptureStorage(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.directory=Path(self.tmp.name) / "pictures"
        self.store=Storage(self.directory)

    def save(self):
        return self.store.save(lambda p: Path(p).write_bytes(PNG))

    def test_png_private_unique(self):
        first,second=self.save(),self.save()
        self.assertNotEqual(first,second)
        self.assertEqual(first.stat().st_mode&0o777,0o600)
        self.assertEqual(set(self.store.recent()),{first,second})

    def test_newest_first_even_with_same_time_in_filename(self):
        first,second=self.save(),self.save()
        os.utime(first,ns=(10_000,10_000))
        os.utime(second,ns=(20_000,20_000))
        self.assertEqual(self.store.recent(),[second,first])

    def test_hide_persists_without_deleting(self):
        first,second=self.save(),self.save()
        self.store.hide(first)
        self.assertTrue(first.exists())
        self.assertEqual(Storage(self.directory).recent(),[second])

    def test_no_corrupt_or_temp_artifacts(self):
        with self.assertRaises(ValueError):
            self.store.save(lambda p: Path(p).write_bytes(b"not an image"))
        self.assertEqual(list(self.directory.iterdir()),[])

    def test_ignore_link_or_foreign_files(self):
        outside=Path(self.tmp.name) / "LedgeGrab-20261020-054125-1234567890.png"
        outside.write_bytes(PNG)
        link=self.directory/outside.name
        link.symlink_to(outside)
        with self.assertRaises(ValueError): self.store.hide(outside)
        with self.assertRaises(ValueError): self.store.hide(link)
        self.assertEqual(self.store.recent(),[])

    def test_accurate_crop_for_reverse_clamp_and_scale(self):
        self.assertEqual(crop_bounds((290,180),(-10,9),(300,200),(600,400)),(0,18,580,342))
        self.assertEqual(crop_bounds((.5,.5),(5.5,8.5),(100,100),(125,125)),(0,0,7,11))
        self.assertEqual(crop_bounds((15,25),(15,25),(100,100),(200,200)),(30,50,0,0))

class MockBus:
    def __init__(self):
        self.unsubscribed=[]
        self.calls=[]
    def get_unique_name(self): return ":1.876"
    def signal_subscribe(self,*args):
        self.signal=args[6]; self.request_path=args[3]; return 12
    def signal_unsubscribe(self,no): self.unsubscribed.append(no)
    def call(self,*args):
        self.calls.append(args)
        if args[3]=="Screenshot":
            self.options=args[4].unpack()[1]
            args[9](self,None)
    def call_finish(self,_res): return GLib.Variant("(o)",(self.request_path,))
    def response(self,code,uri=None):
        data={"uri":GLib.Variant("s",uri)} if uri else {}
        self.signal(self,None,self.request_path,portal.REQUEST,"Response",GLib.Variant("(ua{sv})",(code,data)))

class PortalFlow(unittest.TestCase):
    def setUp(self):
        self.bus=MockBus()
        self.patch=patch.object(Gio,"bus_get_sync",return_value=self.bus)
        self.patch.start();self.addCleanup(self.patch.stop)
        self.results=[]

    def start(self):
        portal.request_screenshot(lambda uri,error:self.results.append((uri,error)))

    def test_consent_handle_and_file_once(self):
        self.start()
        self.assertTrue(self.bus.options["interactive"])
        self.assertIn(self.bus.options["handle_token"],self.bus.request_path)
        self.bus.response(0,"file:///tmp/shot.png")
        self.bus.response(0,"file:///tmp/duplicate.png")
        self.assertEqual(self.results,[("file:///tmp/shot.png",None)])
        self.assertEqual(self.bus.unsubscribed,[12])

    def test_cancel_and_fail_distinguished(self):
        self.start();self.bus.response(1)
        self.assertEqual(self.results,[(None,None)])
        self.start();self.bus.response(2)
        self.assertIn("did not return",self.results[-1][1])

    def test_missing_portal_fails_gracefully(self):
        with patch.object(self.bus,"call_finish",side_effect=GLib.Error("ServiceUnknown")):
            self.start()
        self.assertIn("unavailable",self.results[0][1])

if __name__=="__main__": unittest.main()
