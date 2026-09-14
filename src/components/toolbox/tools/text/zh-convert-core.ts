import { Converter } from "opencc-js"

export const ZH_DIRECTIONS = ["s2t", "t2s"] as const

export type ZhDirection = (typeof ZH_DIRECTIONS)[number]

const converters: Record<ZhDirection, (text: string) => string> = {
  s2t: Converter({ from: "cn", to: "tw" }),
  t2s: Converter({ from: "tw", to: "cn" }),
}

export function convertZh(text: string, direction: ZhDirection): string {
  if (text === "") return ""
  return converters[direction](text)
}
