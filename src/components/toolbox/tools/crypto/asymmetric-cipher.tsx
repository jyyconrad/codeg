"use client"

import { useCallback, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { useCopiedFlag } from "@/hooks/use-copied-flag"
import { ParamField, ParamPanel, ParamSelect, Warn } from "./crypto-fields"
import { type ByteEncoding, errorMessage } from "./encoding"
import {
  type RsaModulus,
  type RsaOaepHash,
  type RsaPemFormat,
  type RsaSignHash,
  type RsaSignScheme,
  generateRsaKeyPair,
  rsaDecrypt,
  rsaEncrypt,
  rsaSign,
  rsaVerify,
} from "./rsa"
import {
  type Sm2CipherMode,
  generateSm2KeyPair,
  sm2Decrypt,
  sm2Encrypt,
  sm2Sign,
  sm2Verify,
} from "./sm2"

type Family = "rsa" | "sm2"
type Action = "keys" | "encrypt" | "sign"

export default function AsymmetricCipherTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [family, setFamily] = useState<Family>("rsa")
  const [action, setAction] = useState<Action>("encrypt")
  const [direction, setDirection] = useState<"forward" | "reverse">("forward")
  const [publicKey, setPublicKey] = useState("")
  const [privateKey, setPrivateKey] = useState("")
  const [signature, setSignature] = useState("")
  const [result, setResult] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [copied, markCopied] = useCopiedFlag()

  const [rsaBits, setRsaBits] = useState<RsaModulus>(2048)
  const [pemFormat, setPemFormat] = useState<RsaPemFormat>("pkcs8")
  const [oaepHash, setOaepHash] = useState<RsaOaepHash>("SHA-256")
  const [signScheme, setSignScheme] = useState<RsaSignScheme>("pkcs1")
  const [signHash, setSignHash] = useState<RsaSignHash>("SHA-256")

  const [sm2Mode, setSm2Mode] = useState<Sm2CipherMode>("c1c3c2")
  const [sm2Asn1, setSm2Asn1] = useState(false)
  const [sm2Prefix, setSm2Prefix] = useState(true)
  const [sm2Der, setSm2Der] = useState(false)

  const [inputEncoding, setInputEncoding] = useState<ByteEncoding>("utf8")
  const [outputEncoding, setOutputEncoding] = useState<ByteEncoding>("hex")

  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  async function copyPrivateKey() {
    if (!privateKey) return
    try {
      await navigator.clipboard.writeText(privateKey)
      markCopied()
    } catch {
      /* clipboard may be denied */
    }
  }

  async function generateKeys() {
    setBusy(true)
    setError(null)
    try {
      if (family === "rsa") {
        const pair = await generateRsaKeyPair(rsaBits, pemFormat)
        setPublicKey(pair.publicKey)
        setPrivateKey(pair.privateKey)
        setResult(pair.publicKey)
      } else {
        const pair = await generateSm2KeyPair(sm2Prefix)
        setPublicKey(pair.publicKey)
        setPrivateKey(pair.privateKey)
        setResult(pair.publicKey)
      }
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  async function run() {
    setBusy(true)
    setError(null)
    try {
      if (action === "keys") {
        await generateKeys()
        return
      }
      if (family === "rsa") {
        if (action === "encrypt" && direction === "forward") {
          setResult(
            await rsaEncrypt({
              publicKeyPem: publicKey,
              plaintext: input,
              inputEncoding,
              outputEncoding,
              hash: oaepHash,
            })
          )
        } else if (action === "encrypt") {
          setResult(
            await rsaDecrypt({
              privateKeyPem: privateKey,
              ciphertext: input,
              inputEncoding,
              outputEncoding,
              hash: oaepHash,
            })
          )
        } else if (direction === "forward") {
          const sig = await rsaSign({
            privateKeyPem: privateKey,
            message: input,
            messageEncoding: inputEncoding,
            outputEncoding,
            scheme: signScheme,
            hash: signHash,
          })
          setSignature(sig)
          setResult(sig)
        } else {
          const ok = await rsaVerify({
            publicKeyPem: publicKey,
            message: input,
            messageEncoding: inputEncoding,
            signature,
            signatureEncoding: outputEncoding,
            scheme: signScheme,
            hash: signHash,
          })
          setResult(ok ? "Signature valid" : "Signature invalid")
        }
      } else if (action === "encrypt" && direction === "forward") {
        setResult(
          await sm2Encrypt({
            publicKey,
            plaintext: input,
            inputEncoding,
            outputEncoding,
            cipherMode: sm2Mode,
            asn1: sm2Asn1,
          })
        )
      } else if (action === "encrypt") {
        setResult(
          await sm2Decrypt({
            privateKey,
            ciphertext: input,
            inputEncoding,
            outputEncoding,
            cipherMode: sm2Mode,
            asn1: sm2Asn1,
          })
        )
      } else if (direction === "forward") {
        const sig = await sm2Sign({
          privateKey,
          publicKey: publicKey || undefined,
          message: input,
          messageEncoding: inputEncoding,
          der: sm2Der,
        })
        setSignature(sig)
        setResult(sig)
      } else {
        const ok = sm2Verify({
          publicKey,
          message: input,
          messageEncoding: inputEncoding,
          signature,
          der: sm2Der,
        })
        setResult(ok ? "Signature valid" : "Signature invalid")
      }
    } catch (err) {
      setResult("")
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const inputLabel =
    action === "encrypt"
      ? direction === "forward"
        ? "Plaintext"
        : "Ciphertext"
      : "Message"

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      inputLabel={inputLabel}
      result={result}
      error={error}
      cryptoFooter
      example="Hello, Codeg"
      params={
        <div className="flex flex-col gap-3">
          <ParamPanel title={t("params")}>
            <ParamField label="Algorithm">
              <ParamSelect
                value={family}
                onChange={(value) => setFamily(value as Family)}
                options={[
                  { value: "rsa", label: "RSA" },
                  { value: "sm2", label: "SM2" },
                ]}
              />
            </ParamField>
            <ParamField label="Action">
              <ParamSelect
                value={action}
                onChange={(value) => setAction(value as Action)}
                options={[
                  { value: "keys", label: "Generate keys" },
                  { value: "encrypt", label: "Encrypt / Decrypt" },
                  { value: "sign", label: "Sign / Verify" },
                ]}
              />
            </ParamField>
            {action !== "keys" ? (
              <ParamField label={action === "encrypt" ? "Direction" : "Mode"}>
                <ParamSelect
                  value={direction}
                  onChange={(value) =>
                    setDirection(value as "forward" | "reverse")
                  }
                  options={
                    action === "encrypt"
                      ? [
                          { value: "forward", label: "Encrypt" },
                          { value: "reverse", label: "Decrypt" },
                        ]
                      : [
                          { value: "forward", label: "Sign" },
                          { value: "reverse", label: "Verify" },
                        ]
                  }
                />
              </ParamField>
            ) : null}
            {family === "rsa" && action === "keys" ? (
              <>
                <ParamField label="Modulus">
                  <ParamSelect
                    value={String(rsaBits)}
                    onChange={(value) =>
                      setRsaBits(Number(value) as RsaModulus)
                    }
                    options={[
                      { value: "2048", label: "2048" },
                      { value: "4096", label: "4096" },
                    ]}
                  />
                </ParamField>
                <ParamField label="PEM format">
                  <ParamSelect
                    value={pemFormat}
                    onChange={(value) => setPemFormat(value as RsaPemFormat)}
                    options={[
                      { value: "pkcs8", label: "PKCS#8 / SPKI" },
                      { value: "pkcs1", label: "PKCS#1" },
                    ]}
                  />
                </ParamField>
              </>
            ) : null}
            {family === "rsa" && action === "encrypt" ? (
              <ParamField label="OAEP hash">
                <ParamSelect
                  value={oaepHash}
                  onChange={(value) => setOaepHash(value as RsaOaepHash)}
                  options={[
                    { value: "SHA-256", label: "SHA-256" },
                    { value: "SHA-1", label: "SHA-1" },
                  ]}
                />
              </ParamField>
            ) : null}
            {family === "rsa" && action === "sign" ? (
              <>
                <ParamField label="Signature scheme">
                  <ParamSelect
                    value={signScheme}
                    onChange={(value) => setSignScheme(value as RsaSignScheme)}
                    options={[
                      { value: "pkcs1", label: "RSASSA-PKCS1-v1_5" },
                      { value: "pss", label: "PSS" },
                    ]}
                  />
                </ParamField>
                <ParamField label="Hash">
                  <ParamSelect
                    value={signHash}
                    onChange={(value) => setSignHash(value as RsaSignHash)}
                    options={[
                      { value: "SHA-256", label: "SHA-256" },
                      { value: "SHA-384", label: "SHA-384" },
                      { value: "SHA-512", label: "SHA-512" },
                      { value: "SHA-1", label: "SHA-1" },
                    ]}
                  />
                </ParamField>
              </>
            ) : null}
            {family === "sm2" && action === "encrypt" ? (
              <>
                <ParamField label="Cipher order">
                  <ParamSelect
                    value={sm2Mode}
                    onChange={(value) => setSm2Mode(value as Sm2CipherMode)}
                    options={[
                      { value: "c1c3c2", label: "C1C3C2" },
                      { value: "c1c2c3", label: "C1C2C3" },
                    ]}
                  />
                </ParamField>
                <label className="flex items-center gap-2 pb-1 text-xs text-muted-foreground">
                  <input
                    type="checkbox"
                    checked={sm2Asn1}
                    onChange={(event) => setSm2Asn1(event.target.checked)}
                  />
                  ASN.1
                </label>
              </>
            ) : null}
            {family === "sm2" && action === "keys" ? (
              <label className="flex items-center gap-2 pb-1 text-xs text-muted-foreground">
                <input
                  type="checkbox"
                  checked={sm2Prefix}
                  onChange={(event) => setSm2Prefix(event.target.checked)}
                />
                Uncompressed 04 prefix
              </label>
            ) : null}
            {family === "sm2" && action === "sign" ? (
              <label className="flex items-center gap-2 pb-1 text-xs text-muted-foreground">
                <input
                  type="checkbox"
                  checked={sm2Der}
                  onChange={(event) => setSm2Der(event.target.checked)}
                />
                ASN.1 / DER signature
              </label>
            ) : null}
            {action !== "keys" ? (
              <>
                <ParamField label="Input encoding">
                  <ParamSelect
                    value={inputEncoding}
                    onChange={(value) =>
                      setInputEncoding(value as ByteEncoding)
                    }
                    options={[
                      { value: "utf8", label: "UTF-8" },
                      { value: "hex", label: "Hex" },
                      { value: "base64", label: "Base64" },
                    ]}
                  />
                </ParamField>
                <ParamField label="Output encoding">
                  <ParamSelect
                    value={outputEncoding}
                    onChange={(value) =>
                      setOutputEncoding(value as ByteEncoding)
                    }
                    options={[
                      { value: "hex", label: "Hex" },
                      { value: "base64", label: "Base64" },
                      { value: "utf8", label: "UTF-8" },
                    ]}
                  />
                </ParamField>
              </>
            ) : null}
            <Button
              type="button"
              size="sm"
              disabled={busy}
              onClick={() => void run()}
            >
              {busy ? "Working…" : action === "keys" ? "Generate" : "Run"}
            </Button>
          </ParamPanel>
          <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
            <label className="flex flex-col gap-1">
              <span className="text-xs font-medium text-muted-foreground">
                Public key
              </span>
              <Textarea
                value={publicKey}
                onChange={(event) => setPublicKey(event.target.value)}
                className="min-h-24 font-mono text-xs"
                autoComplete="off"
              />
            </label>
            <label className="flex flex-col gap-1">
              <span className="text-xs font-medium text-muted-foreground">
                Private key
              </span>
              <Textarea
                value={privateKey}
                onChange={(event) => setPrivateKey(event.target.value)}
                className="min-h-24 font-mono text-xs"
                autoComplete="off"
              />
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="self-start"
                disabled={!privateKey}
                onClick={() => void copyPrivateKey()}
              >
                {copied ? t("copied") : "Copy private key"}
              </Button>
            </label>
          </div>
          {action === "sign" && direction === "reverse" ? (
            <label className="flex flex-col gap-1">
              <span className="text-xs font-medium text-muted-foreground">
                Signature
              </span>
              <Textarea
                value={signature}
                onChange={(event) => setSignature(event.target.value)}
                className="min-h-20 font-mono text-xs"
              />
            </label>
          ) : null}
          {family === "rsa" && action === "encrypt" ? (
            <Warn>
              Encryption uses RSA-OAEP. PKCS#1 v1.5 encryption is legacy and not
              offered.
            </Warn>
          ) : null}
        </div>
      }
    />
  )
}
