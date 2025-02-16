#!/usr/bin/env nix-shell
#!nix-shell -i python3 -p "python3.withPackages (ps: [ ps.z3 ])"
# Use z3 to solve UTF-8 validation DFA for offset and transition table,
# in order to encode transition table into u32.
# We minimize the output variables in the solution to make it deterministic.
# Ref: <https://gist.github.com/dougallj/166e326de6ad4cf2c94be97a204c025f>
#
# It is expected to find a solution in <60s on a modern machine, and the
# solution is appended to the end of this file.
from z3 import *

STATE_CNT = 10

# The transition table.
# A value X on column Y means state Y should transition to state X on some
# input bytes. We assign state 0 as ERROR and state 1 as ACCEPT (initial).
# Eg. first line: for input byte 00..=7F, transition S1 -> S1, others -> S0.
TRANSITIONS = [
    # 0  1  2  3  4  5  6  7  8  9
    # First bytes
    ((0, 1, 0, 0, 0, 0, 0, 0, 0, 0), "00-7F"),
    ((0, 2, 0, 0, 0, 0, 0, 0, 0, 0), "C2-DF"),
    ((0, 3, 0, 0, 0, 0, 0, 0, 0, 0), "E0"),
    ((0, 4, 0, 0, 0, 0, 0, 0, 0, 0), "E1-EC, EE-EF"),
    ((0, 5, 0, 0, 0, 0, 0, 0, 0, 0), "ED"),
    ((0, 6, 0, 0, 0, 0, 0, 0, 0, 0), "F0"),
    ((0, 7, 0, 0, 0, 0, 0, 0, 0, 0), "F1-F3"),
    ((0, 8, 0, 0, 0, 0, 0, 0, 0, 0), "F4"),
    # Continuation bytes
    ((0, 0, 1, 0, 2, 2, 0, 9, 9, 2), "80-8F"),
    ((0, 0, 1, 0, 2, 2, 9, 9, 0, 2), "90-9F"),
    ((0, 0, 1, 2, 2, 0, 9, 9, 0, 2), "A0-BF"),
    # Illegal
    ((0, 0, 0, 0, 0, 0, 0, 0, 0, 0), "C0-C1, F5-FF"),
]

o = Optimize()
offsets = [BitVec(f"o{i}", 32) for i in range(STATE_CNT)]
trans_table = [BitVec(f"t{i}", 32) for i in range(len(TRANSITIONS))]
# When we transition into ERROR, we want to get the length of the error sequence, which is dependent to
# the previous state. Here we want to encoded the error length into the previous transition table entry.
# So even though all error states have `state & 31 == ST_ERROR`, we can distinguish different failing modes
# via inspecting some higher bits `state >> offset_error_len_discr`.
offset_error_len_discr = BitVec("oe", 32)
cvt_error_len = BitVec("te", 32)

# Add some guiding constraints to make solving faster.
o.add(offsets[0] == 0)
o.add(trans_table[-1] == 0)
o.add(offset_error_len_discr < 32)

for i in range(len(offsets)):
    o.add(offsets[i] < 32)
    for j in range(i):
        o.add(offsets[i] != offsets[j])
for trans, (targets, _) in zip(trans_table, TRANSITIONS):
    for src, tgt in enumerate(targets):
        new_st = LShR(trans, offsets[src])
        o.add((new_st & 31) == offsets[tgt])

        error_len_discr = LShR(new_st, offset_error_len_discr)
        error_len = LShR(cvt_error_len, error_len_discr) & 3
        if tgt == 2:
            if src == 1:
                o.add(error_len == 1)
            elif src in [3, 4, 5]:
                o.add(error_len == 2)
            elif src == 9:
                o.add(error_len == 3)
        elif tgt == 9:
            o.add(error_len == 2)

# Minimize ordered outputs to get a unique solution.
goal = Concat(*offsets, offset_error_len_discr, *trans_table, cvt_error_len)
o.minimize(goal)
assert o.check() == sat
model = o.model()

print("Offset[]= ", [model[i].as_long() for i in offsets])
print("Offset[error_len_discr]= ", model[offset_error_len_discr].as_long())
print("error_len converter= {0:#10x} // {0:032b}".format(model[cvt_error_len].as_long()))
print("Transitions:")
for (_, label), v in zip(TRANSITIONS, [model[i].as_long() for i in trans_table]):
    print(f"{label:14} => {v:#10x}, // {v:032b}")

# Output should be deterministic:
# Offset[]=  [0, 6, 16, 19, 13, 25, 11, 18, 24, 1]
# Offset[error_len_discr]=  25
# error_len converter=    0x30302 // 00000000000000110000001100000010
# Transitions:
# 00-7F          =>      0x180, // 00000000000000000000000110000000
# C2-DF          => 0x80000400, // 10000000000000000000010000000000
# E0             =>      0x4c0, // 00000000000000000000010011000000
# E1-EC, EE-EF   =>      0x340, // 00000000000000000000001101000000
# ED             =>      0x640, // 00000000000000000000011001000000
# F0             =>      0x2c0, // 00000000000000000000001011000000
# F1-F3          =>      0x480, // 00000000000000000000010010000000
# F4             =>      0x600, // 00000000000000000000011000000000
# 80-8F          => 0x21060020, // 00100001000001100000000000100000
# 90-9F          => 0x20060820, // 00100000000001100000100000100000
# A0-BF          => 0x40860820, // 01000000100001100000100000100000
# C0-C1, F5-FF   =>        0x0, // 00000000000000000000000000000000
