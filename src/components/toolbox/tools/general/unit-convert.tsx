"use client"

import { useCallback, useMemo, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolboxSelect } from "@/components/toolbox/toolbox-controls"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Input } from "@/components/ui/input"
import { TOOL_LABEL_CLASS } from "@/components/toolbox/tools/image/image-io"
import {
  convertUnit,
  formatConversion,
  getUnit,
  listUnitCategories,
  listUnits,
  parseUnitInput,
  type UnitCategory,
} from "./unit-convert.core"

export default function UnitConvertTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("1")
  const [category, setCategory] = useState<UnitCategory>("length")
  const [fromId, setFromId] = useState("m")
  const [toId, setToId] = useState("km")

  const applyPending = useCallback((value: string) => {
    setInput(value)
    const parsed = parseUnitInput(value)
    if (!parsed?.unitId) return
    const unit = getUnit(parsed.unitId)
    if (!unit) return
    setCategory(unit.category)
    setFromId(unit.id)
    const others = listUnits(unit.category).filter(
      (item) => item.id !== unit.id
    )
    if (others[0]) setToId(others[0].id)
  }, [])
  useToolPendingInput(applyPending)

  const units = listUnits(category)

  const { result, error } = useMemo(() => {
    const parsed = parseUnitInput(input)
    if (!input.trim()) return { result: "", error: null }
    if (!parsed) return { result: "", error: "Invalid number." }
    try {
      const typed = parsed.unitId ? getUnit(parsed.unitId) : undefined
      const sourceId =
        typed && typed.category === getUnit(fromId)?.category
          ? typed.id
          : fromId
      const converted = convertUnit(parsed.value, sourceId, toId)
      return {
        result: formatConversion(parsed.value, sourceId, toId, converted),
        error: null,
      }
    } catch (err) {
      return {
        result: "",
        error: err instanceof Error ? err.message : "Invalid number.",
      }
    }
  }, [input, fromId, toId])

  function changeCategory(next: UnitCategory) {
    setCategory(next)
    const list = listUnits(next)
    setFromId(list[0]?.id ?? "")
    setToId(list[1]?.id ?? list[0]?.id ?? "")
  }

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      inputLabel={t("input")}
      result={result}
      error={error}
      example="1 km"
      params={
        <div className="flex flex-col gap-2">
          <p className="text-xs font-medium text-muted-foreground">
            {t("params")}
          </p>
          <div className="flex flex-wrap items-end gap-3">
            <div className={TOOL_LABEL_CLASS}>
              Category
              <ToolboxSelect
                value={category}
                onChange={(value) => changeCategory(value as UnitCategory)}
                options={listUnitCategories().map((item) => ({
                  value: item,
                  label: item,
                }))}
                className="w-full"
              />
            </div>
            <div className={TOOL_LABEL_CLASS}>
              From
              <ToolboxSelect
                value={fromId}
                onChange={setFromId}
                options={units.map((unit) => ({
                  value: unit.id,
                  label: unit.id,
                }))}
                className="w-full"
              />
            </div>
            <div className={TOOL_LABEL_CLASS}>
              To
              <ToolboxSelect
                value={toId}
                onChange={setToId}
                options={units.map((unit) => ({
                  value: unit.id,
                  label: unit.id,
                }))}
                className="w-full"
              />
            </div>
          </div>
        </div>
      }
      inputSlot={
        <Input
          value={input}
          onChange={(event) => setInput(event.target.value)}
          className="font-mono"
        />
      }
    />
  )
}
