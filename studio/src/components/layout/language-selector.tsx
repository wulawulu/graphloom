import { Globe } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { activeStudioLocale, setStudioLocale, type StudioLocale } from "@/i18n"

export function LanguageSelector(): React.ReactElement {
  const { t } = useTranslation()
  const locale = activeStudioLocale()
  return (
    <Select value={locale} onValueChange={(value) => void setStudioLocale(value as StudioLocale)}>
      <SelectTrigger aria-label={t("Language")} className="h-8 w-auto min-w-0 gap-1.5 px-2 text-xs">
        <Globe className="size-3.5 shrink-0" />
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="en">{t("English")}</SelectItem>
        <SelectItem value="zh-CN">{t("Simplified Chinese")}</SelectItem>
      </SelectContent>
    </Select>
  )
}
