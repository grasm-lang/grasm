"""The oracle for packages/pycalendar: CPython 3.13's calendar in the C
locale. Integers in, hex UTF-8 text out."""
import calendar
import sys


def hx(s):
    return s.encode().hex() or "-"


def line(f):
    op, a = f[0], [int(x) for x in f[1:]]
    try:
        if op == "fm":
            return hx(calendar.TextCalendar(a[4]).formatmonth(a[0], a[1], a[2], a[3]))
        if op == "fy":
            return hx(calendar.TextCalendar(a[5]).formatyear(a[0], a[1], a[2], a[3], a[4]))
        if op == "hm":
            return hx(calendar.HTMLCalendar(a[3]).formatmonth(a[0], a[1], withyear=a[2] == 1))
        if op == "hy":
            return hx(calendar.HTMLCalendar(a[2]).formatyear(a[0], a[1]))
        if op == "mr":
            return "%d,%d" % calendar.monthrange(a[0], a[1])
        if op == "mc":
            return "/".join(",".join(map(str, w)) for w in calendar.Calendar(a[2]).monthdayscalendar(a[0], a[1]))
        if op == "wd":
            return str(int(calendar.weekday(a[0], a[1], a[2])))
        if op == "ld":
            return str(calendar.leapdays(a[0], a[1]))
        if op == "il":
            return str(calendar.isleap(a[0]))
        return str(calendar.timegm(tuple(a)))
    except ValueError as e:
        return "E" + hx(str(e))


out = [line(l.split(" ")) for l in sys.stdin.read().split("\n") if l]
sys.stdout.write("\n".join(out) + "\n")
