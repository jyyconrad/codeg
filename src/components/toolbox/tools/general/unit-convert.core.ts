export const UNIT_CATEGORIES = [
  "length",
  "mass",
  "temperature",
  "volume",
  "area",
  "time",
  "speed",
  "data",
] as const

export type UnitCategory = (typeof UNIT_CATEGORIES)[number]

export type UnitDef = {
  id: string
  category: UnitCategory
  toCanonical: (n: number) => number
  fromCanonical: (n: number) => number
  aliases: readonly string[]
}

function linear(
  factor: number
): Pick<UnitDef, "toCanonical" | "fromCanonical"> {
  return {
    toCanonical: (n) => n * factor,
    fromCanonical: (n) => n / factor,
  }
}

const UNITS: UnitDef[] = [
  { id: "nm", category: "length", ...linear(1e-9), aliases: ["nanometer"] },
  { id: "μm", category: "length", ...linear(1e-6), aliases: ["um", "micron"] },
  { id: "mm", category: "length", ...linear(1e-3), aliases: ["millimeter"] },
  { id: "cm", category: "length", ...linear(0.01), aliases: ["centimeter"] },
  { id: "m", category: "length", ...linear(1), aliases: ["meter", "meters"] },
  { id: "km", category: "length", ...linear(1000), aliases: ["kilometer"] },
  {
    id: "in",
    category: "length",
    ...linear(0.0254),
    aliases: ["inch", "inches"],
  },
  {
    id: "ft",
    category: "length",
    ...linear(0.3048),
    aliases: ["foot", "feet"],
  },
  { id: "yd", category: "length", ...linear(0.9144), aliases: ["yard"] },
  { id: "mi", category: "length", ...linear(1609.344), aliases: ["mile"] },

  { id: "mg", category: "mass", ...linear(1e-6), aliases: ["milligram"] },
  { id: "g", category: "mass", ...linear(1e-3), aliases: ["gram"] },
  { id: "kg", category: "mass", ...linear(1), aliases: ["kilogram"] },
  { id: "t", category: "mass", ...linear(1000), aliases: ["tonne", "ton"] },
  { id: "oz", category: "mass", ...linear(0.028349523125), aliases: ["ounce"] },
  { id: "lb", category: "mass", ...linear(0.45359237), aliases: ["pound"] },

  {
    id: "°C",
    category: "temperature",
    toCanonical: (n) => n,
    fromCanonical: (n) => n,
    aliases: ["C", "c", "celsius"],
  },
  {
    id: "°F",
    category: "temperature",
    toCanonical: (n) => (n - 32) * (5 / 9),
    fromCanonical: (n) => n * (9 / 5) + 32,
    aliases: ["F", "f", "fahrenheit"],
  },
  {
    id: "K",
    category: "temperature",
    toCanonical: (n) => n - 273.15,
    fromCanonical: (n) => n + 273.15,
    aliases: ["kelvin"],
  },

  {
    id: "mL",
    category: "volume",
    ...linear(0.001),
    aliases: ["ml", "milliliter"],
  },
  { id: "L", category: "volume", ...linear(1), aliases: ["l", "liter"] },
  { id: "m³", category: "volume", ...linear(1000), aliases: ["m3"] },
  {
    id: "gal",
    category: "volume",
    ...linear(3.785411784),
    aliases: ["gallon"],
  },

  { id: "mm²", category: "area", ...linear(1e-6), aliases: ["mm2"] },
  { id: "cm²", category: "area", ...linear(1e-4), aliases: ["cm2"] },
  { id: "m²", category: "area", ...linear(1), aliases: ["m2"] },
  { id: "km²", category: "area", ...linear(1e6), aliases: ["km2"] },
  { id: "ha", category: "area", ...linear(1e4), aliases: ["hectare"] },
  { id: "acre", category: "area", ...linear(4046.8564224), aliases: [] },

  { id: "ms", category: "time", ...linear(0.001), aliases: ["millisecond"] },
  { id: "s", category: "time", ...linear(1), aliases: ["sec", "second"] },
  { id: "min", category: "time", ...linear(60), aliases: ["minute"] },
  { id: "h", category: "time", ...linear(3600), aliases: ["hr", "hour"] },
  { id: "d", category: "time", ...linear(86400), aliases: ["day"] },

  { id: "m/s", category: "speed", ...linear(1), aliases: ["mps"] },
  { id: "km/h", category: "speed", ...linear(1000 / 3600), aliases: ["kph"] },
  { id: "mph", category: "speed", ...linear(0.44704), aliases: [] },
  { id: "kn", category: "speed", ...linear(0.514444), aliases: ["knot"] },

  { id: "bit", category: "data", ...linear(1 / 8), aliases: ["bits"] },
  { id: "B", category: "data", ...linear(1), aliases: ["byte", "bytes"] },
  { id: "kB", category: "data", ...linear(1000), aliases: ["KB"] },
  { id: "KiB", category: "data", ...linear(1024), aliases: ["kib"] },
  { id: "MB", category: "data", ...linear(1e6), aliases: ["mb"] },
  { id: "MiB", category: "data", ...linear(1024 ** 2), aliases: ["mib"] },
  { id: "GB", category: "data", ...linear(1e9), aliases: ["gb"] },
  { id: "GiB", category: "data", ...linear(1024 ** 3), aliases: ["gib"] },
  { id: "TB", category: "data", ...linear(1e12), aliases: ["tb"] },
  { id: "TiB", category: "data", ...linear(1024 ** 4), aliases: ["tib"] },
]

const BY_ID = new Map(UNITS.map((unit) => [unit.id, unit]))
const BY_ALIAS = new Map<string, UnitDef>()
for (const unit of UNITS) {
  BY_ALIAS.set(unit.id.toLowerCase(), unit)
  for (const alias of unit.aliases) {
    BY_ALIAS.set(alias.toLowerCase(), unit)
  }
}

export function listUnitCategories(): readonly UnitCategory[] {
  return UNIT_CATEGORIES
}

export function listUnits(category: UnitCategory): readonly UnitDef[] {
  return UNITS.filter((unit) => unit.category === category)
}

export function getUnit(id: string): UnitDef | undefined {
  return BY_ID.get(id)
}

export function resolveUnitAlias(raw: string): UnitDef | undefined {
  return BY_ALIAS.get(raw.trim().toLowerCase()) ?? BY_ID.get(raw.trim())
}

export function convertUnit(
  value: number,
  fromId: string,
  toId: string
): number {
  if (!Number.isFinite(value)) {
    throw new Error("Invalid number.")
  }
  const from = getUnit(fromId)
  const to = getUnit(toId)
  if (!from || !to) throw new Error("Unknown unit.")
  if (from.category !== to.category) {
    throw new Error("Incompatible units.")
  }
  return to.fromCanonical(from.toCanonical(value))
}

export function formatUnitNumber(n: number): string {
  if (!Number.isFinite(n)) return ""
  if (Object.is(n, -0)) return "0"
  if (Number.isInteger(n)) return String(n)
  const abs = Math.abs(n)
  if (abs !== 0 && (abs >= 1e10 || abs < 1e-6)) {
    return n.toExponential(6)
  }
  return String(parseFloat(n.toPrecision(12)))
}

export function parseUnitInput(
  raw: string
): { value: number; unitId?: string } | null {
  const trimmed = raw.trim()
  if (!trimmed) return null
  const match =
    /^([+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?)(?:\s+([^\s]+))?$/.exec(
      trimmed
    )
  if (!match) return null
  const value = Number(match[1])
  if (!Number.isFinite(value)) return null
  if (!match[2]) return { value }
  const unit = resolveUnitAlias(match[2])
  return { value, unitId: unit?.id }
}

export function formatConversion(
  value: number,
  fromId: string,
  toId: string,
  result: number
): string {
  return `${formatUnitNumber(value)} ${fromId} = ${formatUnitNumber(result)} ${toId}`
}
