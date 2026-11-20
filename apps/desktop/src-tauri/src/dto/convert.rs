//! `From` conversions between the desktop DTO enums and their `hatoba-core` / `hatoba-ssh`
//! counterparts.

use hatoba_ipc::mirror_enum;

use super::{
    CursorChoice, DeployStep, DeployStepStatus, DeployWorkerAction, ForwardKind, GenerateAlgorithm,
    KeyAlgorithm, ProxyKind, RightClick, ThemeChoice,
};

mirror_enum!(
    ForwardKind <=> hatoba_core::model::ForwardKind:
    Local,
    Remote,
    Dynamic,
);
mirror_enum!(
    ProxyKind <=> hatoba_core::model::ProxyKind:
    Socks5,
    Http,
);
mirror_enum!(
    ProxyKind => hatoba_ssh::ProxyKind:
    Socks5,
    Http,
);
mirror_enum!(
    KeyAlgorithm <=> hatoba_core::model::KeyAlgorithm:
    Ed25519,
    Ecdsa,
    Rsa,
);
mirror_enum!(
    hatoba_ssh::KeyAlgorithm => KeyAlgorithm:
    Ed25519,
    Ecdsa,
    Rsa,
);
mirror_enum!(
    GenerateAlgorithm => hatoba_ssh::GenerateKind:
    Ed25519,
    Rsa => Rsa4096,
);
mirror_enum!(
    ThemeChoice <=> hatoba_core::model::ThemeMode:
    Light,
    Dark,
    System,
);
mirror_enum!(
    CursorChoice <=> hatoba_core::model::CursorStyle:
    Bar,
    Underline,
    Block,
);
mirror_enum!(
    RightClick <=> hatoba_core::model::RightClick:
    Menu,
    CopyPaste,
);
mirror_enum!(
    hatoba_core::sync::deploy::Step => DeployStep:
    Verify,
    Inspect,
    CreateDatabase,
    Migrate,
    Upload,
    SetupToken,
    Route,
    Wait,
);
mirror_enum!(
    hatoba_core::sync::deploy::StepStatus => DeployStepStatus:
    Running,
    Done,
    Skipped,
);
mirror_enum!(
    hatoba_core::sync::deploy::WorkerPlan => DeployWorkerAction:
    Create,
    Update,
    HasVault,
    Foreign,
);
