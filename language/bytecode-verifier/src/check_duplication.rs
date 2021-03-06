// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! This module implements a checker for verifying that each vector in a CompiledModule contains
//! distinct values. Successful verification implies that an index in vector can be used to
//! uniquely name the entry at that index. Additionally, the checker also verifies the
//! following:
//! - struct and field definitions are consistent
//! - the handles in struct and function definitions point to the self module index
//! - all struct and function handles pointing to the self module index have a definition
use libra_types::vm_error::StatusCode;
use std::{collections::HashSet, hash::Hash};
use vm::{
    access::ModuleAccess,
    errors::{verification_error, VMResult},
    file_format::{CompiledModule, FunctionHandleIndex, StructFieldInformation, StructHandleIndex},
    IndexKind,
};

pub struct DuplicationChecker<'a> {
    module: &'a CompiledModule,
}

impl<'a> DuplicationChecker<'a> {
    pub fn verify(module: &'a CompiledModule) -> VMResult<()> {
        let checker = Self { module };
        // Identifiers
        if let Some(idx) = Self::first_duplicate_element(checker.module.identifiers()) {
            return Err(verification_error(
                IndexKind::Identifier,
                idx,
                StatusCode::DUPLICATE_ELEMENT,
            ));
        }
        // Constants
        if let Some(idx) = Self::first_duplicate_element(checker.module.constant_pool()) {
            return Err(verification_error(
                IndexKind::ConstantPool,
                idx,
                StatusCode::DUPLICATE_ELEMENT,
            ));
        }
        // Signatures
        if let Some(idx) = Self::first_duplicate_element(checker.module.signatures()) {
            return Err(verification_error(
                IndexKind::Signature,
                idx,
                StatusCode::DUPLICATE_ELEMENT,
            ));
        }
        // ModuleHandles
        if let Some(idx) = Self::first_duplicate_element(checker.module.module_handles()) {
            return Err(verification_error(
                IndexKind::ModuleHandle,
                idx,
                StatusCode::DUPLICATE_ELEMENT,
            ));
        }
        // StructHandles - module and name define uniqueness
        if let Some(idx) = Self::first_duplicate_element(
            checker
                .module
                .struct_handles()
                .iter()
                .map(|x| (x.module, x.name)),
        ) {
            return Err(verification_error(
                IndexKind::StructHandle,
                idx,
                StatusCode::DUPLICATE_ELEMENT,
            ));
        }
        // FunctionHandles - module and name define uniqueness
        if let Some(idx) = Self::first_duplicate_element(
            checker
                .module
                .function_handles()
                .iter()
                .map(|x| (x.module, x.name)),
        ) {
            return Err(verification_error(
                IndexKind::FunctionHandle,
                idx,
                StatusCode::DUPLICATE_ELEMENT,
            ));
        }
        // FieldHandles
        if let Some(idx) = Self::first_duplicate_element(checker.module.field_handles()) {
            return Err(verification_error(
                IndexKind::FieldHandle,
                idx,
                StatusCode::DUPLICATE_ELEMENT,
            ));
        }
        // StructInstantiations
        if let Some(idx) = Self::first_duplicate_element(checker.module.struct_instantiations()) {
            return Err(verification_error(
                IndexKind::StructDefInstantiation,
                idx,
                StatusCode::DUPLICATE_ELEMENT,
            ));
        }
        // FunctionInstantiations
        if let Some(idx) = Self::first_duplicate_element(checker.module.function_instantiations()) {
            return Err(verification_error(
                IndexKind::FunctionInstantiation,
                idx,
                StatusCode::DUPLICATE_ELEMENT,
            ));
        }
        // FieldInstantiations
        if let Some(idx) = Self::first_duplicate_element(checker.module.field_instantiations()) {
            return Err(verification_error(
                IndexKind::FieldInstantiation,
                idx,
                StatusCode::DUPLICATE_ELEMENT,
            ));
        }
        // StructDefinition - contained StructHandle defines uniqueness
        if let Some(idx) = Self::first_duplicate_element(
            checker.module.struct_defs().iter().map(|x| x.struct_handle),
        ) {
            return Err(verification_error(
                IndexKind::StructDefinition,
                idx,
                StatusCode::DUPLICATE_ELEMENT,
            ));
        }
        // FunctionDefinition - contained FunctionHandle defines uniqueness
        if let Some(idx) =
            Self::first_duplicate_element(checker.module.function_defs().iter().map(|x| x.function))
        {
            return Err(verification_error(
                IndexKind::FunctionDefinition,
                idx,
                StatusCode::DUPLICATE_ELEMENT,
            ));
        }
        // Acquires in function declarations contain unique struct definitions
        for (idx, function_def) in checker.module.function_defs().iter().enumerate() {
            let acquires = function_def.acquires_global_resources.iter();
            if Self::first_duplicate_element(acquires).is_some() {
                return Err(verification_error(
                    IndexKind::FunctionDefinition,
                    idx,
                    StatusCode::DUPLICATE_ACQUIRES_RESOURCE_ANNOTATION_ERROR,
                ));
            }
        }
        // Field names in structs must be unique
        for (struct_idx, struct_def) in checker.module.struct_defs().iter().enumerate() {
            let fields = match &struct_def.field_information {
                StructFieldInformation::Native => continue,
                StructFieldInformation::Declared(fields) => fields,
            };
            if fields.is_empty() {
                return Err(verification_error(
                    IndexKind::StructDefinition,
                    struct_idx,
                    StatusCode::ZERO_SIZED_STRUCT,
                ));
            }
            if let Some(idx) = Self::first_duplicate_element(fields.iter().map(|x| x.name)) {
                return Err(verification_error(
                    IndexKind::FieldDefinition,
                    idx,
                    StatusCode::DUPLICATE_ELEMENT,
                ));
            }
        }
        // Check that each struct definition is pointing to the self module
        if let Some(idx) = checker.module.struct_defs().iter().position(|x| {
            checker.module.struct_handle_at(x.struct_handle).module
                != checker.module.self_handle_idx()
        }) {
            return Err(verification_error(
                IndexKind::StructDefinition,
                idx,
                StatusCode::INVALID_MODULE_HANDLE,
            ));
        }
        // Check that each function definition is pointing to the self module
        if let Some(idx) = checker.module.function_defs().iter().position(|x| {
            checker.module.function_handle_at(x.function).module != checker.module.self_handle_idx()
        }) {
            return Err(verification_error(
                IndexKind::FunctionDefinition,
                idx,
                StatusCode::INVALID_MODULE_HANDLE,
            ));
        }
        // Check that each struct handle in self module is implemented (has a declaration)
        let implemented_struct_handles: HashSet<StructHandleIndex> = checker
            .module
            .struct_defs()
            .iter()
            .map(|x| x.struct_handle)
            .collect();
        if let Some(idx) = (0..checker.module.struct_handles().len()).position(|x| {
            let y = StructHandleIndex::new(x as u16);
            checker.module.struct_handle_at(y).module == checker.module.self_handle_idx()
                && !implemented_struct_handles.contains(&y)
        }) {
            return Err(verification_error(
                IndexKind::StructHandle,
                idx,
                StatusCode::UNIMPLEMENTED_HANDLE,
            ));
        }
        // Check that each function handle in self module is implemented (has a declaration)
        let implemented_function_handles: HashSet<FunctionHandleIndex> = checker
            .module
            .function_defs()
            .iter()
            .map(|x| x.function)
            .collect();
        if let Some(idx) = (0..checker.module.function_handles().len()).position(|x| {
            let y = FunctionHandleIndex::new(x as u16);
            checker.module.function_handle_at(y).module == checker.module.self_handle_idx()
                && !implemented_function_handles.contains(&y)
        }) {
            return Err(verification_error(
                IndexKind::FunctionHandle,
                idx,
                StatusCode::UNIMPLEMENTED_HANDLE,
            ));
        }

        Ok(())
    }

    fn first_duplicate_element<T>(iter: T) -> Option<usize>
    where
        T: IntoIterator,
        T::Item: Eq + Hash,
    {
        let mut uniq = HashSet::new();
        for (i, x) in iter.into_iter().enumerate() {
            if !uniq.insert(x) {
                return Some(i);
            }
        }
        None
    }
}
