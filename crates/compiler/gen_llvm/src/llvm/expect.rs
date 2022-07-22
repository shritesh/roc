use crate::llvm::bitcode::{call_bitcode_fn, call_str_bitcode_fn};
use crate::llvm::build::{get_tag_id, tag_pointer_clear_tag_id, Env, FAST_CALL_CONV};
use crate::llvm::build_list::{list_len, load_list_ptr};
use crate::llvm::build_str::str_equal;
use crate::llvm::convert::basic_type_from_layout;
use bumpalo::collections::Vec;
use inkwell::types::BasicType;
use inkwell::values::{
    BasicValue, BasicValueEnum, FunctionValue, IntValue, PointerValue, StructValue,
};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate};
use roc_builtins::bitcode;
use roc_builtins::bitcode::{FloatWidth, IntWidth};
use roc_module::symbol::Symbol;
use roc_mono::layout::{Builtin, Layout, LayoutIds, UnionLayout};
use roc_region::all::Region;

use super::build::{
    dec_binop_with_unchecked, load_roc_value, load_symbol_and_layout, use_roc_value, Scope,
};
use super::convert::argument_type_from_union_layout;

pub(crate) fn clone_to_shared_memory<'a, 'ctx, 'env>(
    env: &Env<'a, 'ctx, 'env>,
    scope: &Scope<'a, 'ctx>,
    layout_ids: &mut LayoutIds<'a>,
    condition: Symbol,
    region: Region,
    lookups: &[Symbol],
) {
    let func = env
        .module
        .get_function(bitcode::UTILS_EXPECT_FAILED_START)
        .unwrap();

    let call_result = env
        .builder
        .build_call(func, &[], "call_expect_start_failed");

    let original_ptr = call_result
        .try_as_basic_value()
        .left()
        .unwrap()
        .into_pointer_value();

    let mut ptr = original_ptr;

    {
        let value = env
            .context
            .i32_type()
            .const_int(region.start().offset as _, false);

        let cast_ptr = env.builder.build_pointer_cast(
            ptr,
            value.get_type().ptr_type(AddressSpace::Generic),
            "to_store_pointer",
        );

        env.builder.build_store(cast_ptr, value);

        // let increment = layout.stack_size(env.target_info);
        let increment = 4;
        let increment = env.ptr_int().const_int(increment as _, false);

        ptr = unsafe { env.builder.build_gep(ptr, &[increment], "increment_ptr") };
    }

    {
        let value = env
            .context
            .i32_type()
            .const_int(region.end().offset as _, false);

        let cast_ptr = env.builder.build_pointer_cast(
            ptr,
            value.get_type().ptr_type(AddressSpace::Generic),
            "to_store_pointer",
        );

        env.builder.build_store(cast_ptr, value);

        // let increment = layout.stack_size(env.target_info);
        let increment = 4;
        let increment = env.ptr_int().const_int(increment as _, false);

        ptr = unsafe { env.builder.build_gep(ptr, &[increment], "increment_ptr") };
    }

    {
        let region_bytes: u32 = unsafe { std::mem::transmute(condition.module_id()) };
        let value = env.context.i32_type().const_int(region_bytes as _, false);

        let cast_ptr = env.builder.build_pointer_cast(
            ptr,
            value.get_type().ptr_type(AddressSpace::Generic),
            "to_store_pointer",
        );

        env.builder.build_store(cast_ptr, value);

        // let increment = layout.stack_size(env.target_info);
        let increment = 4;
        let increment = env.ptr_int().const_int(increment as _, false);

        ptr = unsafe { env.builder.build_gep(ptr, &[increment], "increment_ptr") };
    }

    let mut offset = env.ptr_int().const_int(12, false);
    let mut ptr = original_ptr;

    for lookup in lookups.iter() {
        let (value, layout) = load_symbol_and_layout(scope, lookup);

        offset = build_clone(
            env,
            layout_ids,
            ptr,
            offset,
            value,
            *layout,
            WhenRecursive::Unreachable,
        );
    }
}

#[derive(Clone, Debug)]
enum WhenRecursive<'a> {
    Unreachable,
    Loop(UnionLayout<'a>),
}

fn build_clone<'a, 'ctx, 'env>(
    env: &Env<'a, 'ctx, 'env>,
    _layout_ids: &mut LayoutIds<'a>,
    ptr: PointerValue<'ctx>,
    offset: IntValue<'ctx>,
    value: BasicValueEnum<'ctx>,
    layout: Layout<'a>,
    when_recursive: WhenRecursive<'a>,
) -> IntValue<'ctx> {
    match layout {
        Layout::Builtin(builtin) => {
            build_clone_builtin(env, ptr, offset, value, builtin, when_recursive)
        }

        /*
        Layout::Struct { field_layouts, .. } => build_struct_eq(
            env,
            layout_ids,
            field_layouts,
            when_recursive,
            lhs_val.into_struct_value(),
            rhs_val.into_struct_value(),
        ),

        Layout::LambdaSet(_) => unreachable!("cannot compare closures"),

        Layout::Union(union_layout) => build_tag_eq(
            env,
            layout_ids,
            when_recursive,
            union_layout,
            lhs_val,
            rhs_val,
        ),

        Layout::Boxed(inner_layout) => build_box_eq(
            env,
            layout_ids,
            when_recursive,
            lhs_layout,
            inner_layout,
            lhs_val,
            rhs_val,
        ),

        Layout::RecursivePointer => match when_recursive {
            WhenRecursive::Unreachable => {
                unreachable!("recursion pointers should never be compared directly")
            }

            WhenRecursive::Loop(union_layout) => {
                let layout = Layout::Union(union_layout);

                let bt = basic_type_from_layout(env, &layout);

                // cast the i64 pointer to a pointer to block of memory
                let field1_cast = env
                    .builder
                    .build_bitcast(lhs_val, bt, "i64_to_opaque")
                    .into_pointer_value();

                let field2_cast = env
                    .builder
                    .build_bitcast(rhs_val, bt, "i64_to_opaque")
                    .into_pointer_value();

                build_tag_eq(
                    env,
                    layout_ids,
                    WhenRecursive::Loop(union_layout),
                    &union_layout,
                    field1_cast.into(),
                    field2_cast.into(),
                )
            }
        },
        */
        _ => todo!(),
    }
}

fn build_copy<'a, 'ctx, 'env>(
    env: &Env<'a, 'ctx, 'env>,
    ptr: PointerValue<'ctx>,
    offset: IntValue<'ctx>,
    value: BasicValueEnum<'ctx>,
) -> IntValue<'ctx> {
    let ptr = unsafe {
        env.builder
            .build_in_bounds_gep(ptr, &[offset], "at_current_offset")
    };

    let ptr_type = value.get_type().ptr_type(AddressSpace::Generic);
    let ptr = env
        .builder
        .build_pointer_cast(ptr, ptr_type, "cast_ptr_type");

    env.builder.build_store(ptr, value);

    let width = value.get_type().size_of().unwrap();
    env.builder.build_int_add(offset, width, "new_offset")
}

fn build_clone_builtin<'a, 'ctx, 'env>(
    env: &Env<'a, 'ctx, 'env>,
    ptr: PointerValue<'ctx>,
    offset: IntValue<'ctx>,
    value: BasicValueEnum<'ctx>,
    builtin: Builtin<'a>,
    when_recursive: WhenRecursive<'a>,
) -> IntValue<'ctx> {
    use Builtin::*;

    match builtin {
        Int(_) | Float(_) | Bool | Decimal => build_copy(env, ptr, offset, value),

        Builtin::Str => {
            //

            call_bitcode_fn(
                env,
                &[ptr.into(), offset.into(), value],
                bitcode::STR_CLONE_TO,
            )
            .into_int_value()
        }
        Builtin::List(elem) => todo!(),
    }
}
