"""Read a KiCad netlist (.net) into plain Python, with no KiCad dependency.

Only what the layout needs: each component's reference, value and footprint,
and each net's list of (reference, pin) nodes.
"""


def _tokens(text):
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c in "()":
            yield c
            i += 1
        elif c.isspace():
            i += 1
        elif c == '"':
            j = i + 1
            buf = []
            while text[j] != '"':
                if text[j] == "\\":
                    j += 1
                buf.append(text[j])
                j += 1
            yield ("s", "".join(buf))
            i = j + 1
        else:
            j = i
            while j < n and not text[j].isspace() and text[j] not in "()":
                j += 1
            yield ("s", text[i:j])
            i = j


def parse_sexpr(text):
    stack = [[]]
    for t in _tokens(text):
        if t == "(":
            stack.append([])
        elif t == ")":
            done = stack.pop()
            stack[-1].append(done)
        else:
            stack[-1].append(t[1])
    return stack[0][0]


def _find(node, key):
    return [c for c in node if isinstance(c, list) and c and c[0] == key]


def _val(node, key, default=None):
    f = _find(node, key)
    return f[0][1] if f and len(f[0]) > 1 else default


def read(path):
    with open(path) as fh:
        root = parse_sexpr(fh.read())
    comps = {}
    for comp in _find(_find(root, "components")[0], "comp"):
        ref = _val(comp, "ref")
        comps[ref] = {
            "value": _val(comp, "value"),
            "footprint": _val(comp, "footprint"),
            "dnp": any(p for p in _find(comp, "property")
                       if _val(p, "name") == "dnp"),
            "exclude_from_bom": any(p for p in _find(comp, "property")
                                    if _val(p, "name") == "exclude_from_bom"),
        }
    nets = {}
    for net in _find(_find(root, "nets")[0], "net"):
        name = _val(net, "name")
        nets[name] = [(_val(n, "ref"), _val(n, "pin"))
                      for n in _find(net, "node")]
    return comps, nets
