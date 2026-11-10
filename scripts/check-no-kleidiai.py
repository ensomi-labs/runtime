#!/usr/bin/env python3
"""Conservative no-KleidiAI/no-SME policy gate; NOT a general CPU compatibility proof.
Guards against the removed kernels. The Android app carries the same file; change both together.
Usage: python3 check-no-kleidiai.py /path/to/llvm/bin libensomi_runtime_ffi.so
Exit 0: no forbidden evidence; 1: forbidden evidence; 2: invalid input/tool failure.
Does not modify the input or create/delete temporary files.
"""
import collections, hashlib, pathlib, re, subprocess, sys

def main():
    if len(sys.argv) != 3:
        raise ValueError(__doc__)
    llvm, so = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    data = so.read_bytes()
    if len(data) < 64 or data[:6] != b'\x7fELF\x02\x01' or int.from_bytes(data[18:20], 'little') != 183:
        raise ValueError('expected ELF64 little-endian AArch64 input')
    # Deliberately do not reject the harmless UseKleidiAI(false) stub name.
    # Match source paths and real kernel API prefixes, also present in the stripped positive control.
    string_hits = sorted(set(m.group().decode('ascii') for m in re.finditer(
        rb'(?:[^\x00-\x1f\x7f-\xff]*kleidiai-src[^\x00-\x1f\x7f-\xff]*|kai_(?:run|kernel|get)_[A-Za-z0-9_]+)', data)))
    cmd = [str(llvm/'llvm-objdump'), '-d', '--mattr=+sme,+sme2', str(so)]
    # Validate executable section presence; objdump on a non-code ELF must not silently pass.
    sections = subprocess.run([str(llvm/'llvm-readelf'), '-SW', str(so)], capture_output=True, text=True, check=True)
    if not re.search(r'\s\.text\s+PROGBITS\b.*\bAX\b', sections.stdout):
        raise ValueError('missing executable .text section')
    # stderr to an OS pipe is safe here: llvm normally emits none. communicate drains both.
    dis = subprocess.run(cmd, capture_output=True, text=True, check=True)
    if dis.stderr.strip():
        raise ValueError('disassembler diagnostic: '+dis.stderr)
    counts = collections.Counter(); hits = []; instruction_count = 0
    for line in dis.stdout.splitlines():
        m = re.match(r'\s*([0-9a-f]+):\s+([0-9a-f]{8})\s+(\S+)(.*)', line)
        if not m:
            continue
        instruction_count += 1
        mnemonic, operands = m[3], m[4].split('//', 1)[0]
        # Streaming control, streaming vector length, matrix state, predicate-as-counter.
        forbidden = mnemonic in {'smstart','smstop','rdsvl','addsvl','addspl','cntsb','cntsh','cntsw','cntsd'}
        forbidden |= bool(re.search(r'\b(?:za(?:\d+[hv]?)?|zt0|pn(?:[89]|1[0-5]))(?:\b|\.)', operands))
        forbidden |= bool(re.search(r'\b(?:SVCR|SMCR_EL\d|TPIDR2_EL0)\b', operands, re.I))
        if forbidden:
            counts[mnemonic] += 1
            hits.append(line.strip())
    if not instruction_count:
        raise ValueError('no decoded instruction lines')
    print('input='+str(so))
    print('sha256='+hashlib.sha256(data).hexdigest())
    print('command='+' '.join(cmd))
    print('kernel_strings='+str(len(string_hits)))
    print('forbidden_instruction_lines='+str(len(hits))+' mnemonics='+str(dict(counts)))
    for line in string_hits[:3]+hits[:8]:
        print('evidence: '+line)
    bad = bool(string_hits or hits)
    print('FAIL: removed-kernel/SME policy evidence present' if bad else 'PASS: no removed-kernel/SME policy evidence found')
    return 1 if bad else 0

if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.CalledProcessError) as e:
        print('ERROR (fail closed): '+str(e), file=sys.stderr)
        sys.exit(2)
