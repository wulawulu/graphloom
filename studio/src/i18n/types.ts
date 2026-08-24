import type { en } from "@/i18n/en"

/** Recursively mirrors the English resource while allowing localized leaf values. */
export type TranslationResource = {
  readonly [Key in keyof typeof en]: ResourceNode<(typeof en)[Key]>
}

/** Stable semantic key accepted by Studio presentation adapters. */
export type StudioTranslationKey = ResourceKey<typeof en>

type ResourceNode<Node> = Node extends string
  ? string
  : { readonly [Key in keyof Node]: ResourceNode<Node[Key]> }

type ResourceKey<Node> = {
  [Key in keyof Node & string]: Node[Key] extends string
    ? Key
    : `${Key}.${ResourceKey<Node[Key]>}`
}[keyof Node & string]
