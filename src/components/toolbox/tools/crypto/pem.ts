import { CryptoToolError, decodeBase64, encodeBase64 } from "./encoding"

export function pemEncode(label: string, der: Uint8Array): string {
  const b64 = encodeBase64(der)
  const lines = b64.match(/.{1,64}/g) ?? []
  return `-----BEGIN ${label}-----\n${lines.join("\n")}\n-----END ${label}-----`
}

export function pemDecode(pem: string): { label: string; der: Uint8Array } {
  const match = pem
    .trim()
    .match(/-----BEGIN ([A-Z0-9 ]+)-----([\s\S]*?)-----END \1-----/)
  if (!match) {
    throw new CryptoToolError("illegal-encoding", "Value is not a PEM block.")
  }
  return { label: match[1]!, der: decodeBase64(match[2]!) }
}

function derLength(n: number): Uint8Array {
  if (n < 0x80) return new Uint8Array([n])
  if (n < 0x100) return new Uint8Array([0x81, n])
  if (n < 0x10000) return new Uint8Array([0x82, (n >> 8) & 0xff, n & 0xff])
  return new Uint8Array([0x83, (n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff])
}

function concat(parts: Uint8Array[]): Uint8Array {
  const total = parts.reduce((sum, part) => sum + part.length, 0)
  const out = new Uint8Array(total)
  let offset = 0
  for (const part of parts) {
    out.set(part, offset)
    offset += part.length
  }
  return out
}

function derTlv(tag: number, value: Uint8Array): Uint8Array {
  return concat([new Uint8Array([tag]), derLength(value.length), value])
}

export function derSequence(...parts: Uint8Array[]): Uint8Array {
  return derTlv(0x30, concat(parts))
}

const RSA_ALG_ID = Uint8Array.from([
  0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01,
  0x05, 0x00,
])

export function wrapPkcs1PublicToSpki(pkcs1: Uint8Array): Uint8Array {
  return derSequence(
    RSA_ALG_ID,
    derTlv(0x03, concat([new Uint8Array([0x00]), pkcs1]))
  )
}

export function wrapPkcs1PrivateToPkcs8(pkcs1: Uint8Array): Uint8Array {
  return derSequence(
    Uint8Array.from([0x02, 0x01, 0x00]),
    RSA_ALG_ID,
    derTlv(0x04, pkcs1)
  )
}

interface DerNode {
  tag: number
  value: Uint8Array
}

function readDer(
  data: Uint8Array,
  offset = 0
): { node: DerNode; next: number } {
  if (offset >= data.length) {
    throw new CryptoToolError("illegal-encoding", "Truncated DER.")
  }
  const tag = data[offset]!
  let cursor = offset + 1
  if (cursor >= data.length) {
    throw new CryptoToolError("illegal-encoding", "Truncated DER.")
  }
  let length = data[cursor]!
  cursor += 1
  if (length > 0x80) {
    const count = length & 0x7f
    length = 0
    for (let i = 0; i < count; i += 1) {
      length = (length << 8) | data[cursor]!
      cursor += 1
    }
  }
  const value = data.subarray(cursor, cursor + length)
  if (value.length !== length) {
    throw new CryptoToolError("illegal-encoding", "Truncated DER.")
  }
  return { node: { tag, value }, next: cursor + length }
}

function findTag(seq: Uint8Array, tag: number): Uint8Array | null {
  let offset = 0
  while (offset < seq.length) {
    const { node, next } = readDer(seq, offset)
    if (node.tag === tag) return node.value
    offset = next
  }
  return null
}

export function unwrapSpkiToPkcs1(spki: Uint8Array): Uint8Array {
  const { node } = readDer(spki)
  if (node.tag !== 0x30) {
    throw new CryptoToolError("illegal-encoding", "SPKI is not a SEQUENCE.")
  }
  const bitString = findTag(node.value, 0x03)
  if (!bitString || bitString.length < 1) {
    throw new CryptoToolError(
      "illegal-encoding",
      "SPKI is missing the public key."
    )
  }
  return bitString.subarray(1)
}

export function unwrapPkcs8ToPkcs1(pkcs8: Uint8Array): Uint8Array {
  const { node } = readDer(pkcs8)
  if (node.tag !== 0x30) {
    throw new CryptoToolError("illegal-encoding", "PKCS#8 is not a SEQUENCE.")
  }
  const octet = findTag(node.value, 0x04)
  if (!octet) {
    throw new CryptoToolError(
      "illegal-encoding",
      "PKCS#8 is missing the private key."
    )
  }
  return octet
}

export function rsaPublicDerFromPem(pem: string): Uint8Array {
  const { label, der } = pemDecode(pem)
  if (label === "RSA PUBLIC KEY") return wrapPkcs1PublicToSpki(der)
  if (label === "PUBLIC KEY") return der
  throw new CryptoToolError(
    "illegal-encoding",
    `Unexpected PEM label ${label} for an RSA public key.`
  )
}

export function rsaPrivateDerFromPem(pem: string): Uint8Array {
  const { label, der } = pemDecode(pem)
  if (label === "RSA PRIVATE KEY") return wrapPkcs1PrivateToPkcs8(der)
  if (label === "PRIVATE KEY") return der
  throw new CryptoToolError(
    "illegal-encoding",
    `Unexpected PEM label ${label} for an RSA private key.`
  )
}

export function formatRsaPublicPem(spki: Uint8Array, pkcs1: boolean): string {
  const der = pkcs1 ? unwrapSpkiToPkcs1(spki) : spki
  return pemEncode(pkcs1 ? "RSA PUBLIC KEY" : "PUBLIC KEY", der)
}

export function formatRsaPrivatePem(pkcs8: Uint8Array, pkcs1: boolean): string {
  const der = pkcs1 ? unwrapPkcs8ToPkcs1(pkcs8) : pkcs8
  return pemEncode(pkcs1 ? "RSA PRIVATE KEY" : "PRIVATE KEY", der)
}
