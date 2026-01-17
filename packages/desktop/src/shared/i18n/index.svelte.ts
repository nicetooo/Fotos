/**
 * i18n Module
 * Provides internationalization support for the application
 */

import { getContext, setContext } from 'svelte';
import type { Locale, Translations, I18nContext } from './types';
import { en } from './en';
import { zh } from './zh';
import { ja } from './ja';
import { ko } from './ko';
import { fr } from './fr';
import { de } from './de';
import { es } from './es';
import { pt } from './pt';
import { ru } from './ru';
import { it } from './it';
import { ar } from './ar';
import { nl } from './nl';
import { pl } from './pl';
import { tr } from './tr';
import { vi } from './vi';
import { th } from './th';
import { id } from './id';

export type { Locale, Translations, I18nContext };
export { en, zh, ja, ko, fr, de, es, pt, ru, it, ar, nl, pl, tr, vi, th, id };

const I18N_CONTEXT_KEY = Symbol('i18n');
const LOCALE_STORAGE_KEY = 'footos-locale';

/**
 * All available translations
 */
export const translations: Record<Locale, Translations> = {
    en,
    zh,
    ja,
    ko,
    fr,
    de,
    es,
    pt,
    ru,
    it,
    ar,
    nl,
    pl,
    tr,
    vi,
    th,
    id,
};

/**
 * Supported locales with display names
 */
export const locales: { code: Locale; name: string; nativeName: string }[] = [
    { code: 'en', name: 'English', nativeName: 'English' },
    { code: 'zh', name: 'Chinese', nativeName: '中文' },
    { code: 'ja', name: 'Japanese', nativeName: '日本語' },
    { code: 'ko', name: 'Korean', nativeName: '한국어' },
    { code: 'fr', name: 'French', nativeName: 'Français' },
    { code: 'de', name: 'German', nativeName: 'Deutsch' },
    { code: 'es', name: 'Spanish', nativeName: 'Español' },
    { code: 'pt', name: 'Portuguese', nativeName: 'Português' },
    { code: 'ru', name: 'Russian', nativeName: 'Русский' },
    { code: 'it', name: 'Italian', nativeName: 'Italiano' },
    { code: 'ar', name: 'Arabic', nativeName: 'العربية' },
    { code: 'nl', name: 'Dutch', nativeName: 'Nederlands' },
    { code: 'pl', name: 'Polish', nativeName: 'Polski' },
    { code: 'tr', name: 'Turkish', nativeName: 'Türkçe' },
    { code: 'vi', name: 'Vietnamese', nativeName: 'Tiếng Việt' },
    { code: 'th', name: 'Thai', nativeName: 'ไทย' },
    { code: 'id', name: 'Indonesian', nativeName: 'Bahasa Indonesia' },
];

/**
 * Detect system locale
 */
export function detectSystemLocale(): Locale {
    if (typeof navigator === 'undefined') return 'en';

    const browserLang = navigator.language || (navigator as any).userLanguage || 'en';
    const lang = browserLang.toLowerCase().split('-')[0];

    // Map browser language codes to supported locales
    const localeMap: Record<string, Locale> = {
        'en': 'en',
        'zh': 'zh',
        'ja': 'ja',
        'ko': 'ko',
        'fr': 'fr',
        'de': 'de',
        'es': 'es',
        'pt': 'pt',
        'ru': 'ru',
        'it': 'it',
        'ar': 'ar',
        'nl': 'nl',
        'pl': 'pl',
        'tr': 'tr',
        'vi': 'vi',
        'th': 'th',
        'id': 'id',
    };

    return localeMap[lang] || 'en';
}

/**
 * Valid locale codes for type checking
 */
const validLocales = new Set<string>(['en', 'zh', 'ja', 'ko', 'fr', 'de', 'es', 'pt', 'ru', 'it', 'ar', 'nl', 'pl', 'tr', 'vi', 'th', 'id']);

/**
 * Get saved locale from localStorage or detect from system
 */
export function getSavedLocale(): Locale {
    if (typeof localStorage !== 'undefined') {
        const saved = localStorage.getItem(LOCALE_STORAGE_KEY);
        if (saved && validLocales.has(saved)) {
            return saved as Locale;
        }
    }
    return detectSystemLocale();
}

/**
 * Save locale to localStorage
 */
export function saveLocale(locale: Locale): void {
    if (typeof localStorage !== 'undefined') {
        localStorage.setItem(LOCALE_STORAGE_KEY, locale);
    }
}

/**
 * Get translations for a locale
 */
export function getTranslations(locale: Locale): Translations {
    return translations[locale] || translations.en;
}

/**
 * Set i18n context (call from root component)
 */
export function setI18nContext(context: I18nContext): void {
    setContext(I18N_CONTEXT_KEY, context);
}

/**
 * Get i18n context (call from child components)
 */
export function getI18nContext(): I18nContext {
    return getContext<I18nContext>(I18N_CONTEXT_KEY);
}

/**
 * Create i18n state for use in components
 */
export function createI18nState() {
    let locale = $state<Locale>(getSavedLocale());
    let t = $derived(getTranslations(locale));

    function setLocale(newLocale: Locale) {
        locale = newLocale;
        saveLocale(newLocale);
    }

    return {
        get locale() { return locale; },
        get t() { return t; },
        setLocale,
    };
}
