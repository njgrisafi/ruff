use ruff_diagnostics::{AlwaysAutofixableViolation, Diagnostic, Edit, Fix, Violation};
use ruff_macros::{derive_message_formats, violation};
use ruff_python_ast::call_path::CallPath;
use ruff_python_ast::{
    self as ast, Arguments, Constant, Expr, Operator, ParameterWithDefault, Parameters, Stmt,
    UnaryOp,
};
use ruff_python_semantic::{ScopeKind, SemanticModel};
use ruff_source_file::Locator;
use ruff_text_size::Ranged;

use crate::checkers::ast::Checker;
use crate::importer::ImportRequest;
use crate::registry::AsRule;
use crate::rules::flake8_pyi::rules::TypingModule;
use crate::settings::types::PythonVersion;

#[violation]
pub struct TypedArgumentDefaultInStub;

impl AlwaysAutofixableViolation for TypedArgumentDefaultInStub {
    #[derive_message_formats]
    fn message(&self) -> String {
        format!("Only simple default values allowed for typed arguments")
    }

    fn autofix_title(&self) -> String {
        "Replace default value with `...`".to_string()
    }
}

#[violation]
pub struct ArgumentDefaultInStub;

impl AlwaysAutofixableViolation for ArgumentDefaultInStub {
    #[derive_message_formats]
    fn message(&self) -> String {
        format!("Only simple default values allowed for arguments")
    }

    fn autofix_title(&self) -> String {
        "Replace default value with `...`".to_string()
    }
}

#[violation]
pub struct AssignmentDefaultInStub;

impl AlwaysAutofixableViolation for AssignmentDefaultInStub {
    #[derive_message_formats]
    fn message(&self) -> String {
        format!("Only simple default values allowed for assignments")
    }

    fn autofix_title(&self) -> String {
        "Replace default value with `...`".to_string()
    }
}

#[violation]
pub struct UnannotatedAssignmentInStub {
    name: String,
}

impl Violation for UnannotatedAssignmentInStub {
    #[derive_message_formats]
    fn message(&self) -> String {
        let UnannotatedAssignmentInStub { name } = self;
        format!("Need type annotation for `{name}`")
    }
}

#[violation]
pub struct UnassignedSpecialVariableInStub {
    name: String,
}

/// ## What it does
/// Checks that `__all__`, `__match_args__`, and `__slots__` variables are
/// assigned to values when defined in stub files.
///
/// ## Why is this bad?
/// Special variables like `__all__` have the same semantics in stub files
/// as they do in Python modules, and so should be consistent with their
/// runtime counterparts.
///
/// ## Example
/// ```python
/// __all__: list[str]
/// ```
///
/// Use instead:
/// ```python
/// __all__: list[str] = ["foo", "bar"]
/// ```
impl Violation for UnassignedSpecialVariableInStub {
    #[derive_message_formats]
    fn message(&self) -> String {
        let UnassignedSpecialVariableInStub { name } = self;
        format!("`{name}` in a stub file must have a value, as it has the same semantics as `{name}` at runtime")
    }
}

/// ## What it does
/// Checks for type alias definitions that are not annotated with
/// `typing.TypeAlias`.
///
/// ## Why is this bad?
/// In Python, a type alias is defined by assigning a type to a variable (e.g.,
/// `Vector = list[float]`).
///
/// It's best to annotate type aliases with the `typing.TypeAlias` type to
/// make it clear that the statement is a type alias declaration, as opposed
/// to a normal variable assignment.
///
/// ## Example
/// ```python
/// Vector = list[float]
/// ```
///
/// Use instead:
/// ```python
/// from typing import TypeAlias
///
/// Vector: TypeAlias = list[float]
/// ```
#[violation]
pub struct TypeAliasWithoutAnnotation {
    module: TypingModule,
    name: String,
    value: String,
}

impl AlwaysAutofixableViolation for TypeAliasWithoutAnnotation {
    #[derive_message_formats]
    fn message(&self) -> String {
        let TypeAliasWithoutAnnotation {
            module,
            name,
            value,
        } = self;
        format!("Use `{module}.TypeAlias` for type alias, e.g., `{name}: TypeAlias = {value}`")
    }

    fn autofix_title(&self) -> String {
        "Add `TypeAlias` annotation".to_string()
    }
}

fn is_allowed_negated_math_attribute(call_path: &CallPath) -> bool {
    matches!(call_path.as_slice(), ["math", "inf" | "e" | "pi" | "tau"])
}

fn is_allowed_math_attribute(call_path: &CallPath) -> bool {
    matches!(
        call_path.as_slice(),
        ["math", "inf" | "nan" | "e" | "pi" | "tau"]
            | [
                "sys",
                "stdin"
                    | "stdout"
                    | "stderr"
                    | "version"
                    | "version_info"
                    | "platform"
                    | "executable"
                    | "prefix"
                    | "exec_prefix"
                    | "base_prefix"
                    | "byteorder"
                    | "maxsize"
                    | "hexversion"
                    | "winver"
            ]
    )
}

fn is_valid_default_value_with_annotation(
    default: &Expr,
    allow_container: bool,
    locator: &Locator,
    semantic: &SemanticModel,
) -> bool {
    match default {
        Expr::Constant(_) => {
            return true;
        }
        Expr::List(ast::ExprList { elts, .. })
        | Expr::Tuple(ast::ExprTuple { elts, .. })
        | Expr::Set(ast::ExprSet { elts, range: _ }) => {
            return allow_container
                && elts.len() <= 10
                && elts
                    .iter()
                    .all(|e| is_valid_default_value_with_annotation(e, false, locator, semantic));
        }
        Expr::Dict(ast::ExprDict {
            keys,
            values,
            range: _,
        }) => {
            return allow_container
                && keys.len() <= 10
                && keys.iter().zip(values).all(|(k, v)| {
                    k.as_ref().is_some_and(|k| {
                        is_valid_default_value_with_annotation(k, false, locator, semantic)
                    }) && is_valid_default_value_with_annotation(v, false, locator, semantic)
                });
        }
        Expr::UnaryOp(ast::ExprUnaryOp {
            op: UnaryOp::USub,
            operand,
            range: _,
        }) => {
            match operand.as_ref() {
                // Ex) `-1`, `-3.14`, `2j`
                Expr::Constant(ast::ExprConstant {
                    value: Constant::Int(..) | Constant::Float(..) | Constant::Complex { .. },
                    ..
                }) => return true,
                // Ex) `-math.inf`, `-math.pi`, etc.
                Expr::Attribute(_) => {
                    if semantic
                        .resolve_call_path(operand)
                        .as_ref()
                        .is_some_and(is_allowed_negated_math_attribute)
                    {
                        return true;
                    }
                }
                _ => {}
            }
        }
        Expr::BinOp(ast::ExprBinOp {
            left,
            op: Operator::Add | Operator::Sub,
            right,
            range: _,
        }) => {
            // Ex) `1 + 2j`, `1 - 2j`, `-1 - 2j`, `-1 + 2j`
            if let Expr::Constant(ast::ExprConstant {
                value: Constant::Complex { .. },
                ..
            }) = right.as_ref()
            {
                // Ex) `1 + 2j`, `1 - 2j`
                if let Expr::Constant(ast::ExprConstant {
                    value: Constant::Int(..) | Constant::Float(..),
                    ..
                }) = left.as_ref()
                {
                    return locator.slice(left.as_ref()).len() <= 10;
                } else if let Expr::UnaryOp(ast::ExprUnaryOp {
                    op: UnaryOp::USub,
                    operand,
                    range: _,
                }) = left.as_ref()
                {
                    // Ex) `-1 + 2j`, `-1 - 2j`
                    if let Expr::Constant(ast::ExprConstant {
                        value: Constant::Int(..) | Constant::Float(..),
                        ..
                    }) = operand.as_ref()
                    {
                        return locator.slice(operand.as_ref()).len() <= 10;
                    }
                }
            }
        }
        // Ex) `math.inf`, `sys.stdin`, etc.
        Expr::Attribute(_) => {
            if semantic
                .resolve_call_path(default)
                .as_ref()
                .is_some_and(is_allowed_math_attribute)
            {
                return true;
            }
        }
        _ => {}
    }
    false
}

/// Returns `true` if an [`Expr`] appears to be a valid PEP 604 union. (e.g. `int | None`)
fn is_valid_pep_604_union(annotation: &Expr) -> bool {
    /// Returns `true` if an [`Expr`] appears to be a valid PEP 604 union member.
    fn is_valid_pep_604_union_member(value: &Expr) -> bool {
        match value {
            Expr::BinOp(ast::ExprBinOp {
                left,
                op: Operator::BitOr,
                right,
                range: _,
            }) => is_valid_pep_604_union_member(left) && is_valid_pep_604_union_member(right),
            Expr::Name(_)
            | Expr::Subscript(_)
            | Expr::Attribute(_)
            | Expr::Constant(ast::ExprConstant {
                value: Constant::None,
                ..
            }) => true,
            _ => false,
        }
    }

    // The top-level expression must be a bit-or operation.
    let Expr::BinOp(ast::ExprBinOp {
        left,
        op: Operator::BitOr,
        right,
        range: _,
    }) = annotation
    else {
        return false;
    };

    // The left and right operands must be valid union members.
    is_valid_pep_604_union_member(left) && is_valid_pep_604_union_member(right)
}

/// Returns `true` if an [`Expr`] appears to be a valid default value without an annotation.
fn is_valid_default_value_without_annotation(default: &Expr) -> bool {
    matches!(
        default,
        Expr::Call(_)
            | Expr::Name(_)
            | Expr::Attribute(_)
            | Expr::Subscript(_)
            | Expr::Constant(ast::ExprConstant {
                value: Constant::Ellipsis | Constant::None,
                ..
            })
    ) || is_valid_pep_604_union(default)
}

/// Returns `true` if an [`Expr`] appears to be `TypeVar`, `TypeVarTuple`, `NewType`, or `ParamSpec`
/// call.
fn is_type_var_like_call(expr: &Expr, semantic: &SemanticModel) -> bool {
    let Expr::Call(ast::ExprCall { func, .. }) = expr else {
        return false;
    };
    semantic.resolve_call_path(func).is_some_and(|call_path| {
        matches!(
            call_path.as_slice(),
            [
                "typing" | "typing_extensions",
                "TypeVar" | "TypeVarTuple" | "NewType" | "ParamSpec"
            ]
        )
    })
}

/// Returns `true` if this is a "special" assignment which must have a value (e.g., an assignment to
/// `__all__`).
fn is_special_assignment(target: &Expr, semantic: &SemanticModel) -> bool {
    if let Expr::Name(ast::ExprName { id, .. }) = target {
        match id.as_str() {
            "__all__" => semantic.current_scope().kind.is_module(),
            "__match_args__" | "__slots__" => semantic.current_scope().kind.is_class(),
            _ => false,
        }
    } else {
        false
    }
}

/// Returns `true` if this is an assignment to a simple `Final`-annotated variable.
fn is_final_assignment(annotation: &Expr, value: &Expr, semantic: &SemanticModel) -> bool {
    if matches!(value, Expr::Name(_) | Expr::Attribute(_)) {
        if semantic.match_typing_expr(annotation, "Final") {
            return true;
        }
    }
    false
}

/// Returns `true` if the a class is an enum, based on its base classes.
fn is_enum(arguments: Option<&Arguments>, semantic: &SemanticModel) -> bool {
    let Some(Arguments { args: bases, .. }) = arguments else {
        return false;
    };
    return bases.iter().any(|expr| {
        semantic.resolve_call_path(expr).is_some_and(|call_path| {
            matches!(
                call_path.as_slice(),
                [
                    "enum",
                    "Enum" | "Flag" | "IntEnum" | "IntFlag" | "StrEnum" | "ReprEnum"
                ]
            )
        })
    });
}

/// Returns `true` if an [`Expr`] is a value that should be annotated with `typing.TypeAlias`.
///
/// This is relatively conservative, as it's hard to reliably detect whether a right-hand side is a
/// valid type alias. In particular, this function checks for uses of `typing.Any`, `None`,
/// parameterized generics, and PEP 604-style unions.
fn is_annotatable_type_alias(value: &Expr, semantic: &SemanticModel) -> bool {
    matches!(
        value,
        Expr::Subscript(_)
            | Expr::Constant(ast::ExprConstant {
                value: Constant::None,
                ..
            }),
    ) || is_valid_pep_604_union(value)
        || semantic.match_typing_expr(value, "Any")
}

/// PYI011
pub(crate) fn typed_argument_simple_defaults(checker: &mut Checker, parameters: &Parameters) {
    for ParameterWithDefault {
        parameter,
        default,
        range: _,
    } in parameters
        .posonlyargs
        .iter()
        .chain(&parameters.args)
        .chain(&parameters.kwonlyargs)
    {
        let Some(default) = default else {
            continue;
        };
        if parameter.annotation.is_some() {
            if !is_valid_default_value_with_annotation(
                default,
                true,
                checker.locator(),
                checker.semantic(),
            ) {
                let mut diagnostic = Diagnostic::new(TypedArgumentDefaultInStub, default.range());

                if checker.patch(diagnostic.kind.rule()) {
                    diagnostic.set_fix(Fix::suggested(Edit::range_replacement(
                        "...".to_string(),
                        default.range(),
                    )));
                }

                checker.diagnostics.push(diagnostic);
            }
        }
    }
}

/// PYI014
pub(crate) fn argument_simple_defaults(checker: &mut Checker, parameters: &Parameters) {
    for ParameterWithDefault {
        parameter,
        default,
        range: _,
    } in parameters
        .posonlyargs
        .iter()
        .chain(&parameters.args)
        .chain(&parameters.kwonlyargs)
    {
        let Some(default) = default else {
            continue;
        };
        if parameter.annotation.is_none() {
            if !is_valid_default_value_with_annotation(
                default,
                true,
                checker.locator(),
                checker.semantic(),
            ) {
                let mut diagnostic = Diagnostic::new(ArgumentDefaultInStub, default.range());

                if checker.patch(diagnostic.kind.rule()) {
                    diagnostic.set_fix(Fix::suggested(Edit::range_replacement(
                        "...".to_string(),
                        default.range(),
                    )));
                }

                checker.diagnostics.push(diagnostic);
            }
        }
    }
}

/// PYI015
pub(crate) fn assignment_default_in_stub(checker: &mut Checker, targets: &[Expr], value: &Expr) {
    let [target] = targets else {
        return;
    };
    if !target.is_name_expr() {
        return;
    }
    if is_special_assignment(target, checker.semantic()) {
        return;
    }
    if is_type_var_like_call(value, checker.semantic()) {
        return;
    }
    if is_valid_default_value_without_annotation(value) {
        return;
    }
    if is_valid_default_value_with_annotation(value, true, checker.locator(), checker.semantic()) {
        return;
    }

    let mut diagnostic = Diagnostic::new(AssignmentDefaultInStub, value.range());
    if checker.patch(diagnostic.kind.rule()) {
        diagnostic.set_fix(Fix::suggested(Edit::range_replacement(
            "...".to_string(),
            value.range(),
        )));
    }
    checker.diagnostics.push(diagnostic);
}

/// PYI015
pub(crate) fn annotated_assignment_default_in_stub(
    checker: &mut Checker,
    target: &Expr,
    value: &Expr,
    annotation: &Expr,
) {
    if checker
        .semantic()
        .match_typing_expr(annotation, "TypeAlias")
    {
        return;
    }
    if is_special_assignment(target, checker.semantic()) {
        return;
    }
    if is_type_var_like_call(value, checker.semantic()) {
        return;
    }
    if is_final_assignment(annotation, value, checker.semantic()) {
        return;
    }
    if is_valid_default_value_with_annotation(value, true, checker.locator(), checker.semantic()) {
        return;
    }

    let mut diagnostic = Diagnostic::new(AssignmentDefaultInStub, value.range());
    if checker.patch(diagnostic.kind.rule()) {
        diagnostic.set_fix(Fix::suggested(Edit::range_replacement(
            "...".to_string(),
            value.range(),
        )));
    }
    checker.diagnostics.push(diagnostic);
}

/// PYI052
pub(crate) fn unannotated_assignment_in_stub(
    checker: &mut Checker,
    targets: &[Expr],
    value: &Expr,
) {
    let [target] = targets else {
        return;
    };
    let Expr::Name(ast::ExprName { id, .. }) = target else {
        return;
    };
    if is_special_assignment(target, checker.semantic()) {
        return;
    }
    if is_type_var_like_call(value, checker.semantic()) {
        return;
    }
    if is_valid_default_value_without_annotation(value) {
        return;
    }
    if !is_valid_default_value_with_annotation(value, true, checker.locator(), checker.semantic()) {
        return;
    }

    if let ScopeKind::Class(ast::StmtClassDef { arguments, .. }) =
        checker.semantic().current_scope().kind
    {
        if is_enum(arguments.as_deref(), checker.semantic()) {
            return;
        }
    }
    checker.diagnostics.push(Diagnostic::new(
        UnannotatedAssignmentInStub {
            name: id.to_string(),
        },
        value.range(),
    ));
}

/// PYI035
pub(crate) fn unassigned_special_variable_in_stub(
    checker: &mut Checker,
    target: &Expr,
    stmt: &Stmt,
) {
    let Expr::Name(ast::ExprName { id, .. }) = target else {
        return;
    };

    if !is_special_assignment(target, checker.semantic()) {
        return;
    }

    checker.diagnostics.push(Diagnostic::new(
        UnassignedSpecialVariableInStub {
            name: id.to_string(),
        },
        stmt.range(),
    ));
}

/// PYI026
pub(crate) fn type_alias_without_annotation(checker: &mut Checker, value: &Expr, targets: &[Expr]) {
    let [target] = targets else {
        return;
    };

    let Expr::Name(ast::ExprName { id, .. }) = target else {
        return;
    };

    if !is_annotatable_type_alias(value, checker.semantic()) {
        return;
    }

    let module = if checker.settings.target_version >= PythonVersion::Py310 {
        TypingModule::Typing
    } else {
        TypingModule::TypingExtensions
    };

    let mut diagnostic = Diagnostic::new(
        TypeAliasWithoutAnnotation {
            module,
            name: id.to_string(),
            value: checker.generator().expr(value),
        },
        target.range(),
    );
    if checker.patch(diagnostic.kind.rule()) {
        diagnostic.try_set_fix(|| {
            let (import_edit, binding) = checker.importer().get_or_import_symbol(
                &ImportRequest::import(module.as_str(), "TypeAlias"),
                target.start(),
                checker.semantic(),
            )?;
            Ok(Fix::suggested_edits(
                Edit::range_replacement(format!("{id}: {binding}"), target.range()),
                [import_edit],
            ))
        });
    }
    checker.diagnostics.push(diagnostic);
}
