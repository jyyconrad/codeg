import { isLocalDesktop } from "@/lib/platform"
import { getTransport } from "@/lib/transport"

export type ToolboxHashAlgorithm =
  | "MD5"
  | "SHA-1"
  | "SHA-256"
  | "SHA-384"
  | "SHA-512"
  | "SHA3-256"
  | "SM3"

export type ToolboxHashReport = Record<ToolboxHashAlgorithm, string>

export const TOOLBOX_PROGRESS_EVENT = "toolbox://progress"

export interface ToolboxProgress {
  jobId: string
  kind: string
  bytesDone: number
  bytesTotal: number
}

export interface ToolboxCertView {
  subject: string
  issuer: string
  serial: string
  notBefore: string
  notAfter: string
  san: string[]
  fingerprintSha256: string
  signatureAlgorithm: string
}

export interface ToolboxCipherFileParams {
  srcPath: string
  destPath: string
  algorithm: string
  mode: string
  padding: string
  direction: string
  key: string
  keyEncoding: string
  iv: string
  ivEncoding: string
  prependIv: boolean
  passphrase?: string | null
  pbkdf2Iterations?: number | null
  jobId: string
}

export function toolboxRustAvailable(): boolean {
  return isLocalDesktop()
}

export async function listenToolboxProgress(
  handler: (event: ToolboxProgress) => void
): Promise<() => void> {
  return getTransport().subscribe<ToolboxProgress>(
    TOOLBOX_PROGRESS_EVENT,
    handler
  )
}

export async function toolboxCancelJob(jobId: string): Promise<boolean> {
  return getTransport().call<boolean>("toolbox_cancel_job", { jobId })
}

export async function toolboxHashFile(
  path: string,
  jobId: string
): Promise<ToolboxHashReport> {
  return getTransport().call<ToolboxHashReport>("toolbox_hash_file", {
    path,
    jobId,
  })
}

export async function toolboxCipherFile(
  params: ToolboxCipherFileParams
): Promise<void> {
  return getTransport().call<void>(
    "toolbox_cipher_file",
    { params },
    { timeoutMs: 3_600_000 }
  )
}

export async function toolboxBcryptHash(
  password: string,
  cost: number,
  jobId: string
): Promise<string> {
  return getTransport().call<string>(
    "toolbox_bcrypt_hash",
    { password, cost, jobId },
    { timeoutMs: 180_000 }
  )
}

export async function toolboxBcryptVerify(
  password: string,
  hash: string
): Promise<boolean> {
  return getTransport().call<boolean>("toolbox_bcrypt_verify", {
    password,
    hash,
  })
}

export async function toolboxParseCert(input: {
  pem?: string
  path?: string
}): Promise<ToolboxCertView> {
  return getTransport().call<ToolboxCertView>("toolbox_parse_cert", {
    pem: input.pem ?? null,
    path: input.path ?? null,
  })
}

export function formatToolboxHashReport(report: ToolboxHashReport): string {
  const weak = new Set<ToolboxHashAlgorithm>(["MD5", "SHA-1"])
  const names: ToolboxHashAlgorithm[] = [
    "MD5",
    "SHA-1",
    "SHA-256",
    "SHA-384",
    "SHA-512",
    "SHA3-256",
    "SM3",
  ]
  return names
    .map((name) => {
      const note = weak.has(name) ? "  (checksum only, not for passwords)" : ""
      return `${name}${note}\n${report[name]}`
    })
    .join("\n\n")
}
