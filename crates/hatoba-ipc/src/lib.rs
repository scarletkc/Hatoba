//! What the desktop app's crates share over IPC (spec §10): [`error::AppError`], the error every
//! command returns, and the data transfer objects that more than one of them uses. Independent of
//! Tauri, so `hatoba-assistant` builds and tests without it.

pub mod dto;
pub mod error;

/// Implements `From` between unit-variant enums that mirror each other across crate or DTO
/// boundaries: `<=>` in both directions, `=>` in one, with `Left => Right` (or `Left <=> Right`)
/// when variant names differ.
#[macro_export]
macro_rules! mirror_enum {
    ($($a:ident)::+ <=> $($b:ident)::+ : $($rest:tt)+) => {
        $crate::mirror_enum!(@fwd $($a)::+ => $($b)::+; [] $($rest)+);
        $crate::mirror_enum!(@rev $($b)::+ => $($a)::+; [] $($rest)+);
    };
    ($($a:ident)::+ => $($b:ident)::+ : $($rest:tt)+) => {
        $crate::mirror_enum!(@fwd $($a)::+ => $($b)::+; [] $($rest)+);
    };

    (@fwd $from:path => $to:path; [$($arms:tt)*] $va:ident => $vb:ident, $($rest:tt)*) => {
        $crate::mirror_enum!(@fwd $from => $to; [$($arms)* <$from>::$va => Self::$vb,] $($rest)*);
    };
    (@fwd $from:path => $to:path; [$($arms:tt)*] $va:ident => $vb:ident) => {
        $crate::mirror_enum!(@emit $from => $to; [$($arms)* <$from>::$va => Self::$vb,]);
    };
    (@fwd $from:path => $to:path; [$($arms:tt)*] $va:ident <=> $vb:ident, $($rest:tt)*) => {
        $crate::mirror_enum!(@fwd $from => $to; [$($arms)* <$from>::$va => Self::$vb,] $($rest)*);
    };
    (@fwd $from:path => $to:path; [$($arms:tt)*] $va:ident <=> $vb:ident) => {
        $crate::mirror_enum!(@emit $from => $to; [$($arms)* <$from>::$va => Self::$vb,]);
    };
    (@fwd $from:path => $to:path; [$($arms:tt)*] $va:ident, $($rest:tt)*) => {
        $crate::mirror_enum!(@fwd $from => $to; [$($arms)* <$from>::$va => Self::$va,] $($rest)*);
    };
    (@fwd $from:path => $to:path; [$($arms:tt)*] $va:ident) => {
        $crate::mirror_enum!(@emit $from => $to; [$($arms)* <$from>::$va => Self::$va,]);
    };
    (@fwd $from:path => $to:path; [$($arms:tt)*]) => {
        $crate::mirror_enum!(@emit $from => $to; [$($arms)*]);
    };

    (@rev $from:path => $to:path; [$($arms:tt)*] $va:ident <=> $vb:ident, $($rest:tt)*) => {
        $crate::mirror_enum!(@rev $from => $to; [$($arms)* <$from>::$vb => Self::$va,] $($rest)*);
    };
    (@rev $from:path => $to:path; [$($arms:tt)*] $va:ident <=> $vb:ident) => {
        $crate::mirror_enum!(@emit $from => $to; [$($arms)* <$from>::$vb => Self::$va,]);
    };
    (@rev $from:path => $to:path; [$($arms:tt)*] $va:ident, $($rest:tt)*) => {
        $crate::mirror_enum!(@rev $from => $to; [$($arms)* <$from>::$va => Self::$va,] $($rest)*);
    };
    (@rev $from:path => $to:path; [$($arms:tt)*] $va:ident) => {
        $crate::mirror_enum!(@emit $from => $to; [$($arms)* <$from>::$va => Self::$va,]);
    };
    (@rev $from:path => $to:path; [$($arms:tt)*]) => {
        $crate::mirror_enum!(@emit $from => $to; [$($arms)*]);
    };

    (@emit $from:path => $to:path; [$($arms:tt)*]) => {
        impl ::core::convert::From<$from> for $to {
            fn from(value: $from) -> Self {
                match value {
                    $($arms)*
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    mod inner {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Left {
            Alpha,
            Beta,
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Right {
        Alpha,
        Second,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum OneWay {
        First,
        Second,
    }

    mirror_enum!(inner::Left <=> Right: Alpha, Beta <=> Second);
    mirror_enum!(Right => OneWay: Alpha => First, Second,);

    #[test]
    fn converts_bidirectional_and_renamed_variants() {
        assert_eq!(Right::from(inner::Left::Alpha), Right::Alpha);
        assert_eq!(Right::from(inner::Left::Beta), Right::Second);
        assert_eq!(inner::Left::from(Right::Alpha), inner::Left::Alpha);
        assert_eq!(inner::Left::from(Right::Second), inner::Left::Beta);
    }

    #[test]
    fn converts_unidirectional_variants() {
        assert_eq!(OneWay::from(Right::Alpha), OneWay::First);
        assert_eq!(OneWay::from(Right::Second), OneWay::Second);
    }
}
