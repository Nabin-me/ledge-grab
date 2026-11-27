"""Request a user-approved local screenshot through XDG Desktop Portal."""
from uuid import uuid4
from gi.repository import Gio, GLib
BUS = "org.freedesktop.portal.Desktop"
ROOT = "/org/freedesktop/portal/desktop"
SCREENSHOT = "org.freedesktop.portal.Screenshot"
REQUEST = "org.freedesktop.portal.Request"

def request_screenshot(done):
    """Callback receives (uri or None, error message or None); cancellation is not an error."""
    try:
        bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        unique = bus.get_unique_name()
        if not unique:
            raise RuntimeError("No desktop session bus.")
    except (GLib.Error,RuntimeError) as e:
        done(None,str(e))
        return
    token = "ledge_" + uuid4().hex
    sender = unique.removeprefix(":").replace(".","_")
    path = f"{ROOT}/request/{sender}/{token}"
    state = {"sub":0,"timer":0,"done":False}

    def finish(uri=None,error=None):
        if state["done"]:
            return
        state["done"] = True
        if state["sub"]:
            bus.signal_unsubscribe(state["sub"])
        if state["timer"]:
            GLib.source_remove(state["timer"])
        done(uri,error)

    def responded(_bus,_sender,_path,_interface,_name,params):
        code, data = params.unpack()
        if code == 1:
            finish()
        elif code == 0 and isinstance(data.get("uri"),str):
            finish(uri=data["uri"])
        else:
            finish(error="The desktop portal did not return a screenshot.")

    def arrived(conn,res):
        try:
            (returned,) = conn.call_finish(res).unpack()
        except GLib.Error as e:
            finish(error="Screenshot portal unavailable: " + str(e.message))
            return
        if not state["done"] and returned != path:
            finish(error="This screenshot portal is too old for the preview.")

    def expire():
        state["timer"] = 0
        bus.call(BUS,path,REQUEST,"Close",None,None,Gio.DBusCallFlags.NONE,2000,None,None)
        finish(error="Screenshot request timed out.")
        return GLib.SOURCE_REMOVE

    state["sub"] = bus.signal_subscribe(BUS,REQUEST,"Response",path,None,Gio.DBusSignalFlags.NONE,responded)
    state["timer"] = GLib.timeout_add_seconds(120,expire)
    opts = {"handle_token":GLib.Variant("s",token),"interactive":GLib.Variant("b",True)}
    bus.call(BUS,ROOT,SCREENSHOT,"Screenshot",GLib.Variant("(sa{sv})",("",opts)),
             GLib.VariantType.new("(o)"),Gio.DBusCallFlags.NONE,10000,None,arrived)
