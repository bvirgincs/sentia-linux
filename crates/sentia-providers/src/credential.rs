// SPDX-License-Identifier: Apache-2.0

use crate::error::{ProviderError, ProviderErrorKind};
use std::{
    fs::File,
    io::{Read, Take},
    os::fd::{FromRawFd, RawFd},
};

const CREDENTIAL_FD: RawFd = 3;
const MAX_CREDENTIAL_BYTES: u64 = 16_384;

pub(crate) struct SecretBytes(Vec<u8>);

pub(crate) fn harden_process() -> Result<(), ProviderError> {
    // SAFETY: prctl is called with documented integer constants and no pointers.
    let dumpable = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    if dumpable != 0 {
        return Err(ProviderError::process(
            "process_hardening_failed",
            "The worker could not disable process dumps.",
        ));
    }
    // SAFETY: the signal constant and zero-valued trailing arguments match
    // PR_SET_PDEATHSIG. Losing the launcher must terminate the credential owner.
    let parent_death =
        unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) };
    if parent_death != 0 {
        return Err(ProviderError::process(
            "process_hardening_failed",
            "The worker could not bind its lifetime to the launcher.",
        ));
    }
    Ok(())
}

impl SecretBytes {
    pub(crate) fn read_inherited() -> Result<Self, ProviderError> {
        // SAFETY: FD 3 is reserved by the worker launch contract and ownership is
        // transferred to this process. No other code constructs a File from it.
        let file = unsafe { File::from_raw_fd(CREDENTIAL_FD) };
        let mut limited: Take<File> = file.take(MAX_CREDENTIAL_BYTES + 1);
        let mut bytes = Vec::new();
        if limited.read_to_end(&mut bytes).is_err() {
            bytes.fill(0);
            return Err(ProviderError::new(
                ProviderErrorKind::Authentication,
                "credential_unavailable",
                "The provider credential descriptor could not be read.",
                false,
            ));
        }

        if bytes.len() as u64 > MAX_CREDENTIAL_BYTES {
            bytes.fill(0);
            return Err(ProviderError::new(
                ProviderErrorKind::Authentication,
                "credential_too_large",
                "The provider credential exceeds the worker limit.",
                false,
            ));
        }

        while bytes
            .last()
            .is_some_and(|byte| matches!(*byte, b'\n' | b'\r'))
        {
            bytes.pop();
        }
        if bytes.is_empty() || bytes.contains(&0) || bytes.contains(&b'\n') || bytes.contains(&b'\r')
        {
            bytes.fill(0);
            return Err(ProviderError::new(
                ProviderErrorKind::Authentication,
                "invalid_credential",
                "The provider credential descriptor contained invalid data.",
                false,
            ));
        }

        Ok(Self(bytes))
    }

    pub(crate) fn expose(&self) -> &[u8] {
        &self.0
    }

    #[cfg(test)]
    pub(crate) fn for_test(value: &[u8]) -> Self {
        Self(value.to_vec())
    }
}

impl Drop for SecretBytes {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}
