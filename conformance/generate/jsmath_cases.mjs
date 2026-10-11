// Writes (op, a, b, expected) records of Node's Math results for crates/generate/examples/jsmath_check.rs.
// Usage: node jsmath_cases.mjs <out.bin> [count per op]
import { writeFileSync } from 'node:fs';

const out = process.argv[2], n = Number(process.argv[3] ?? 400000);
const ops = [(a) => Math.exp(a), (a) => Math.log(a), (a) => Math.log1p(a), (a) => Math.log2(a), (a) => Math.sin(a), (a) => Math.cos(a), (a, b) => Math.atan2(a, b), (a, b) => Math.hypot(a, b)];
let s = 0x9e3779b97f4a7c15n;
const next = () => { s ^= s << 13n; s &= 0xffffffffffffffffn; s ^= s >> 7n; s ^= s << 17n; s &= 0xffffffffffffffffn; return s; };
const uniform = () => Number(next() >> 11n) / 2 ** 53;
const bits = (() => { const b = new BigUint64Array(1), f = new Float64Array(b.buffer); return () => { b[0] = next(); return f[0]; }; })();
const inputs = [
  [() => (uniform() - 0.5) * 40, () => (uniform() - 0.5) * 1500, bits],
  [() => 2 ** ((uniform() - 0.5) * 80), () => uniform() * 4, () => Math.abs(bits())],
  [() => uniform() * 4 - 0.99, () => 2 ** ((uniform() - 0.5) * 60), () => (uniform() - 0.5) * 1e-3, bits],
  [() => 2 ** ((uniform() - 0.5) * 80), () => uniform() * 4, () => Math.abs(bits())],
  [() => (uniform() - 0.5) * 20, () => (uniform() - 0.5) * 20000, () => (uniform() - 0.5) * 1e6, () => 2 * Math.PI * Math.floor(uniform() * 2000) / 16],
  [() => (uniform() - 0.5) * 20, () => (uniform() - 0.5) * 20000, () => (uniform() - 0.5) * 1e6, () => -2 * Math.PI * Math.floor(uniform() * 1024) / 1024],
  [() => (uniform() - 0.5) * 20, () => bits()],
  [() => (uniform() - 0.5) * 20, () => 2 ** ((uniform() - 0.5) * 40)],
];
const buf = new Float64Array(ops.length * n * 4);
let p = 0;
for (let op = 0; op < ops.length; op++)
  for (let i = 0; i < n; i++) {
    const gens = inputs[op], two = op >= 6;
    const a = gens[i % (gens.length - (two ? 0 : 0))](), b = two ? gens[(i * 7 + 1) % gens.length]() : 0;
    buf[p++] = op; buf[p++] = a; buf[p++] = b; buf[p++] = ops[op](a, b);
  }
writeFileSync(out, Buffer.from(buf.buffer));
console.log(`wrote ${ops.length * n} cases to ${out}`);
