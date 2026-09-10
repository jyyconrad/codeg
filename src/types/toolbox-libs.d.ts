declare module "jsqr" {
  interface QrLocation {
    topRightCorner: { x: number; y: number }
    topLeftCorner: { x: number; y: number }
    bottomRightCorner: { x: number; y: number }
    bottomLeftCorner: { x: number; y: number }
  }
  interface QrResult {
    data: string
    location: QrLocation
  }
  export default function jsQR(
    data: Uint8ClampedArray,
    width: number,
    height: number
  ): QrResult | null
}

declare module "opencc-js" {
  export function Converter(options: {
    from: string
    to: string
  }): (text: string) => string
}
