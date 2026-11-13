"""lldb formatters for Lunar programs.

Load them with

    (lldb) command script import <lunar>/std/debugger/lunar_lldb.py

(or put that line in ~/.lldbinit). They show

* `String` and `str` as their text: "hello";
* owning arrays and slices (`[T; ..+]`, `[T; &]`, ..., or `array$<T>` and
  `slice$<T>` in CodeView) as their elements;
* enums (unions of a `tag` and a member per variant, in the debug
  information) as the active variant and its fields only: `Some(5)`,
  `Rect { w = 2, h = 5 }`.
"""

import re

import lldb

MAX_TEXT = 4096
MAX_ELEMENTS = 10000

TEXT_TYPES = r"^(String|str)$"
ARRAY_TYPES = r"^(\[.+\]|(array|slice)\$<.+>)$"


def _raw(valobj):
    return valobj.GetNonSyntheticValue() if valobj.IsSynthetic() else valobj


def _quote(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t") + '"'


def text_summary(valobj, internal_dict):
    v = _raw(valobj)
    ptr = v.GetChildMemberWithName("data_ptr").GetValueAsUnsigned(0)
    n = v.GetChildMemberWithName("length").GetValueAsUnsigned(0)
    if n == 0:
        return '""'
    if ptr == 0:
        return "<null>"
    err = lldb.SBError()
    data = v.GetProcess().ReadMemory(ptr, min(n, MAX_TEXT), err)
    if not err.Success() or data is None:
        return "<unreadable>"
    return _quote(bytes(data).decode("utf-8", "replace")) + ("..." if n > MAX_TEXT else "")


class ArrayProvider:
    """The elements behind `data_ptr`, `length` of them."""

    def __init__(self, valobj, internal_dict):
        self.valobj = valobj
        self.update()

    def update(self):
        v = _raw(self.valobj)
        self.ptr = v.GetChildMemberWithName("data_ptr")
        self.length = v.GetChildMemberWithName("length").GetValueAsUnsigned(0)
        self.elem = self.ptr.GetType().GetPointeeType()
        self.size = max(self.elem.GetByteSize(), 1)
        return False

    def num_children(self):
        return min(self.length, MAX_ELEMENTS)

    def has_children(self):
        return True

    def get_child_index(self, name):
        try:
            return int(name.lstrip("[").rstrip("]"))
        except ValueError:
            return -1

    def get_child_at_index(self, i):
        addr = self.ptr.GetValueAsUnsigned(0) + i * self.size
        return self.valobj.CreateValueFromAddress("[%d]" % i, addr, self.elem)


def array_summary(valobj, internal_dict):
    v = _raw(valobj)
    return "length=%d" % v.GetChildMemberWithName("length").GetValueAsUnsigned(0)


def is_enum(sbtype, internal_dict):
    """A Lunar enum: a union whose first member is the `tag` enumeration."""
    t = sbtype.GetCanonicalType()
    if t.GetTypeClass() != lldb.eTypeClassUnion or t.GetNumberOfFields() < 1:
        return False
    tag = t.GetFieldAtIndex(0)
    return tag.GetName() == "tag" and tag.GetType().GetCanonicalType().GetTypeClass() == lldb.eTypeClassEnumeration


def _active(valobj):
    v = _raw(valobj)
    name = v.GetChildMemberWithName("tag").GetValue()
    if name is None:
        return None, None
    return name, v.GetChildMemberWithName(name)


class EnumProvider:
    """The fields of the active variant."""

    def __init__(self, valobj, internal_dict):
        self.valobj = valobj
        self.update()

    def update(self):
        self.name, self.variant = _active(self.valobj)
        return False

    def num_children(self):
        if self.variant is None or not self.variant.IsValid():
            return 0
        return self.variant.GetNumChildren()

    def has_children(self):
        return self.num_children() > 0

    def get_child_index(self, name):
        if self.variant is None:
            return -1
        for i in range(self.variant.GetNumChildren()):
            if self.variant.GetChildAtIndex(i).GetName() == name:
                return i
        return -1

    def get_child_at_index(self, i):
        return self.variant.GetChildAtIndex(i)


def enum_summary(valobj, internal_dict):
    name, variant = _active(valobj)
    if name is None:
        return "<invalid tag>"
    if variant is None or not variant.IsValid() or variant.GetNumChildren() == 0:
        return name
    fields = []
    positional = True
    for i in range(variant.GetNumChildren()):
        f = variant.GetChildAtIndex(i)
        if not re.match(r"^__\d+$", f.GetName() or ""):
            positional = False
        shown = f.GetSummary() or f.GetValue()
        if shown is None:
            shown = "{...}"
        fields.append((f.GetName(), shown))
    if positional:
        return "%s(%s)" % (name, ", ".join(s for _, s in fields))
    return "%s { %s }" % (name, ", ".join("%s = %s" % f for f in fields))


def __lldb_init_module(debugger, internal_dict):
    m = __name__
    for cmd in [
        'type summary add -w lunar -F %s.text_summary -x "%s"' % (m, TEXT_TYPES),
        'type synthetic add -w lunar -l %s.ArrayProvider -x "%s"' % (m, ARRAY_TYPES),
        'type summary add -w lunar -F %s.array_summary -e -x "%s"' % (m, ARRAY_TYPES),
        "type synthetic add -w lunar -l %s.EnumProvider --recognizer-function %s.is_enum" % (m, m),
        "type summary add -w lunar -F %s.enum_summary -e --recognizer-function %s.is_enum" % (m, m),
        "type category enable lunar",
    ]:
        debugger.HandleCommand(cmd)
