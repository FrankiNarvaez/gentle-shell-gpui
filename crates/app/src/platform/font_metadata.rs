//! System font metadata for the settings picker, without realizing text-system faces.

use std::collections::HashSet;
use std::sync::OnceLock;

static MONOSPACE: OnceLock<HashSet<String>> = OnceLock::new();

/// Nonblocking, even while the background scan is in progress.
pub(crate) fn cached_monospace() -> Option<&'static HashSet<String>> {
    MONOSPACE.get()
}

/// Call on a background executor. All settings views share this one scan.
pub(crate) fn load_monospace() {
    MONOSPACE.get_or_init(scan_monospace);
}

#[cfg(target_os = "macos")]
fn scan_monospace() -> HashSet<String> {
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFType, TCFType as _};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};
    use core_text::font_collection::{CTFontCollectionRef, create_for_all_families};
    use core_text::font_descriptor::{
        CTFontDescriptor, CTFontDescriptorCopyAttribute, kCTFontFamilyNameAttribute,
        kCTFontMonoSpaceTrait, kCTFontSymbolicTrait, kCTFontTraitsAttribute,
    };

    unsafe extern "C" {
        fn CTFontCollectionCreateMatchingFontDescriptors(
            collection: CTFontCollectionRef,
        ) -> CFArrayRef;
    }

    fn attribute(descriptor: &CTFontDescriptor, key: CFStringRef) -> Option<CFType> {
        // The descriptor and attribute key are valid for the call. Own a
        // non-null Copy result before inspecting its type.
        unsafe {
            let value = CTFontDescriptorCopyAttribute(descriptor.as_concrete_TypeRef(), key);
            (!value.is_null()).then(|| CFType::wrap_under_create_rule(value))
        }
    }

    let collection = create_for_all_families();
    // Like GPUI, take ownership of the Create result without retaining it
    // again (core-text 21's get_descriptors uses the Get rule).
    let descriptors: CFArray<CTFontDescriptor> = unsafe {
        let array = CTFontCollectionCreateMatchingFontDescriptors(collection.as_concrete_TypeRef());
        if array.is_null() {
            return HashSet::new();
        }
        CFArray::wrap_under_create_rule(array)
    };

    descriptors
        .iter()
        .filter_map(|descriptor| {
            // CoreText provides these process-lifetime attribute keys.
            let (traits_key, symbolic_key, family_key) = unsafe {
                (
                    kCTFontTraitsAttribute,
                    kCTFontSymbolicTrait,
                    kCTFontFamilyNameAttribute,
                )
            };
            let traits = attribute(&descriptor, traits_key)?.downcast::<CFDictionary>()?;
            // Only the dictionary type has been checked; validate the value separately.
            let traits = unsafe {
                CFDictionary::<CFString, CFType>::wrap_under_get_rule(traits.as_concrete_TypeRef())
            };
            let symbolic = traits.find(symbolic_key)?.downcast::<CFNumber>()?;
            if symbolic.to_i64()? & i64::from(kCTFontMonoSpaceTrait) == 0 {
                return None;
            }
            let family = attribute(&descriptor, family_key)?.downcast::<CFString>()?;
            let family = family.to_string();
            (!family.trim().is_empty()).then(|| family.to_lowercase())
        })
        .collect()
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn core_text_recognizes_system_menlo_as_monospace() {
        assert!(super::scan_monospace().contains("menlo"));
    }
}

#[cfg(not(target_os = "macos"))]
fn scan_monospace() -> HashSet<String> {
    crate::shared::backdrop::preview::system_font_db()
        .faces()
        .filter(|face| face.monospaced)
        .flat_map(|face| face.families.iter().map(|(name, _)| name.to_lowercase()))
        .collect()
}
