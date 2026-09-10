import {
  CryptoToolError,
  bytesEqual,
  decodeBase64Url,
  decodeUtf8,
  encodeUtf8,
} from "./encoding"
import { hmacBytes } from "./hmac"
import { rsaVerify } from "./rsa"

export interface JwtDecodeResult {
  header: unknown
  payload: unknown
  headerJson: string
  payloadJson: string
  signature: string
  alg: string
}

export interface JwtVerifyResult {
  attempted: boolean
  verified: boolean
  alg: string
  reason: string
}

function parseJsonPart(label: string, raw: string): unknown {
  let bytes: Uint8Array
  try {
    bytes = decodeBase64Url(raw)
  } catch {
    throw new CryptoToolError(
      "illegal-encoding",
      `JWT ${label} is not valid Base64URL.`
    )
  }
  const text = decodeUtf8(bytes)
  try {
    return JSON.parse(text) as unknown
  } catch {
    throw new CryptoToolError(
      "invalid-input",
      `JWT ${label} is not valid JSON.`
    )
  }
}

function pretty(value: unknown): string {
  return JSON.stringify(value, null, 2)
}

export function decodeJwt(token: string): JwtDecodeResult {
  const parts = token.trim().split(".")
  if (parts.length !== 3) {
    throw new CryptoToolError(
      "invalid-input",
      "JWT must have three Base64URL parts (header.payload.signature)."
    )
  }
  const header = parseJsonPart("header", parts[0]!)
  const payload = parseJsonPart("payload", parts[1]!)
  const alg =
    header &&
    typeof header === "object" &&
    "alg" in header &&
    typeof (header as { alg: unknown }).alg === "string"
      ? (header as { alg: string }).alg
      : ""
  return {
    header,
    payload,
    headerJson: pretty(header),
    payloadJson: pretty(payload),
    signature: parts[2] ?? "",
    alg,
  }
}

export function formatJwtDecode(result: JwtDecodeResult): string {
  return [
    "Header",
    result.headerJson,
    "",
    "Payload",
    result.payloadJson,
    "",
    `alg: ${result.alg || "(missing)"}`,
    "Decode does not mean the token is valid.",
  ].join("\n")
}

async function verifyHmacJwt(
  token: string,
  alg: "SHA-256" | "SHA-512",
  secret: Uint8Array
): Promise<boolean> {
  const lastDot = token.lastIndexOf(".")
  const signingInput = encodeUtf8(token.slice(0, lastDot))
  const signature = decodeBase64Url(token.slice(lastDot + 1))
  const mac = await hmacBytes(alg, secret, signingInput)
  return bytesEqual(mac, signature)
}

export async function verifyJwt(params: {
  token: string
  decoded: JwtDecodeResult
  secretOrPem: string
}): Promise<JwtVerifyResult> {
  const alg = params.decoded.alg
  if (!params.secretOrPem.trim()) {
    return {
      attempted: false,
      verified: false,
      alg,
      reason: "No key provided — decode only.",
    }
  }
  try {
    if (alg === "HS256" || alg === "HS512") {
      const secret = encodeUtf8(params.secretOrPem)
      const ok = await verifyHmacJwt(
        params.token.trim(),
        alg === "HS256" ? "SHA-256" : "SHA-512",
        secret
      )
      return {
        attempted: true,
        verified: ok,
        alg,
        reason: ok
          ? "HMAC signature verified."
          : "HMAC signature does not match.",
      }
    }
    if (
      alg === "RS256" ||
      alg === "RS512" ||
      alg === "PS256" ||
      alg === "PS512"
    ) {
      const lastDot = params.token.trim().lastIndexOf(".")
      const signingInput = params.token.trim().slice(0, lastDot)
      const signature = params.token.trim().slice(lastDot + 1)
      const scheme = alg.startsWith("PS") ? "pss" : "pkcs1"
      const hash = alg.endsWith("512") ? "SHA-512" : "SHA-256"
      const ok = await rsaVerify({
        publicKeyPem: params.secretOrPem,
        message: signingInput,
        messageEncoding: "utf8",
        signature,
        signatureEncoding: "base64url",
        scheme,
        hash,
      })
      return {
        attempted: true,
        verified: ok,
        alg,
        reason: ok
          ? "RSA signature verified."
          : "RSA signature does not match.",
      }
    }
    return {
      attempted: true,
      verified: false,
      alg,
      reason: `Algorithm ${alg || "(missing)"} is not supported for verify (HS256/HS512/RS256/RS512/PS256/PS512).`,
    }
  } catch (err) {
    return {
      attempted: true,
      verified: false,
      alg,
      reason: err instanceof Error ? err.message : String(err),
    }
  }
}
