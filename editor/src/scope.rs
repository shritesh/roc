#![allow(clippy::all)]
#![allow(dead_code)]
#![allow(unused_imports)]
use crate::pool::{Pool, PoolStr, PoolVec, ShallowClone};
use crate::types::{Alias, TypeId};
use roc_collections::all::{MutMap, MutSet};
use roc_module::ident::{Ident, Lowercase};
use roc_module::symbol::{IdentIds, ModuleId, Symbol};
use roc_problem::can::RuntimeError;
use roc_region::all::{Located, Region};
use roc_types::subs::{VarStore, Variable};

fn solved_type_to_type_id() -> TypeId {
    todo!()
}

#[derive(Debug)]
pub struct Scope {
    /// All the identifiers in scope, mapped to were they were defined and
    /// the Symbol they resolve to.
    idents: MutMap<Ident, (Symbol, Region)>,

    /// A cache of all the symbols in scope. This makes lookups much
    /// faster when checking for unused defs and unused arguments.
    symbols: MutMap<Symbol, Region>,

    /// The type aliases currently in scope
    aliases: MutMap<Symbol, Alias>,

    /// The current module being processed. This will be used to turn
    /// unqualified idents into Symbols.
    home: ModuleId,
}

impl Scope {
    pub fn new(home: ModuleId, pool: &mut Pool, _var_store: &mut VarStore) -> Scope {
        use roc_types::solved_types::{BuiltinAlias, FreeVars};
        let solved_aliases = roc_types::builtin_aliases::aliases();
        let mut aliases = MutMap::default();

        for (symbol, builtin_alias) in solved_aliases {
            // let BuiltinAlias { region, vars, typ } = builtin_alias;
            let BuiltinAlias { vars, .. } = builtin_alias;

            let free_vars = FreeVars::default();
            let typ = solved_type_to_type_id();
            // roc_types::solved_types::to_type(&typ, &mut free_vars, var_store);

            // make sure to sort these variables to make them line up with the type arguments
            let mut type_variables: Vec<_> = free_vars.unnamed_vars.into_iter().collect();
            type_variables.sort();

            debug_assert_eq!(vars.len(), type_variables.len());
            let variables = PoolVec::with_capacity(vars.len() as u32, pool);

            let it = variables
                .iter_node_ids()
                .zip(vars.iter())
                .zip(type_variables);
            for ((node_id, loc_name), (_, var)) in it {
                // TODO region is ignored, but "fake" anyway. How to resolve?
                let name = PoolStr::new(loc_name.value.as_str(), pool);
                pool[node_id] = (name, var);
            }

            let alias = Alias {
                actual: typ,
                /// We know that builtin aliases have no hiddden variables (e.g. in closures)
                hidden_variables: PoolVec::empty(pool),
                targs: variables,
            };

            aliases.insert(symbol, alias);
        }

        let idents = Symbol::default_in_scope();
        let idents: MutMap<_, _> = idents.into_iter().collect();

        Scope {
            home,
            idents,
            symbols: MutMap::default(),
            aliases,
        }
    }

    pub fn idents(&self) -> impl Iterator<Item = (&Ident, &(Symbol, Region))> {
        self.idents.iter()
    }

    pub fn symbols(&self) -> impl Iterator<Item = (Symbol, Region)> + '_ {
        self.symbols.iter().map(|(x, y)| (*x, *y))
    }

    pub fn contains_ident(&self, ident: &Ident) -> bool {
        self.idents.contains_key(ident)
    }

    pub fn contains_symbol(&self, symbol: Symbol) -> bool {
        self.symbols.contains_key(&symbol)
    }

    pub fn num_idents(&self) -> usize {
        self.idents.len()
    }

    pub fn lookup(&mut self, ident: &Ident, region: Region) -> Result<Symbol, RuntimeError> {
        match self.idents.get(ident) {
            Some((symbol, _)) => Ok(*symbol),
            None => Err(RuntimeError::LookupNotInScope(
                Located {
                    region,
                    value: ident.clone().into(),
                },
                self.idents.keys().map(|v| v.as_ref().into()).collect(),
            )),
        }
    }

    pub fn lookup_alias(&self, symbol: Symbol) -> Option<&Alias> {
        self.aliases.get(&symbol)
    }

    /// Introduce a new ident to scope.
    ///
    /// Returns Err if this would shadow an existing ident, including the
    /// Symbol and Region of the ident we already had in scope under that name.
    pub fn introduce(
        &mut self,
        ident: Ident,
        exposed_ident_ids: &IdentIds,
        all_ident_ids: &mut IdentIds,
        region: Region,
    ) -> Result<Symbol, (Region, Located<Ident>)> {
        match self.idents.get(&ident) {
            Some((_, original_region)) => {
                let shadow = Located {
                    value: ident,
                    region,
                };

                Err((*original_region, shadow))
            }
            None => {
                // If this IdentId was already added previously
                // when the value was exposed in the module header,
                // use that existing IdentId. Otherwise, create a fresh one.
                let ident_id = match exposed_ident_ids.get_id(&ident.as_inline_str()) {
                    Some(ident_id) => *ident_id,
                    None => all_ident_ids.add(ident.clone().into()),
                };

                let symbol = Symbol::new(self.home, ident_id);

                self.symbols.insert(symbol, region);
                self.idents.insert(ident, (symbol, region));

                Ok(symbol)
            }
        }
    }

    /// Ignore an identifier.
    ///
    /// Used for record guards like { x: Just _ }
    pub fn ignore(&mut self, ident: Ident, all_ident_ids: &mut IdentIds) -> Symbol {
        let ident_id = all_ident_ids.add(ident.into());
        Symbol::new(self.home, ident_id)
    }

    /// Import a Symbol from another module into this module's top-level scope.
    ///
    /// Returns Err if this would shadow an existing ident, including the
    /// Symbol and Region of the ident we already had in scope under that name.
    pub fn import(
        &mut self,
        ident: Ident,
        symbol: Symbol,
        region: Region,
    ) -> Result<(), (Symbol, Region)> {
        match self.idents.get(&ident) {
            Some(shadowed) => Err(*shadowed),
            None => {
                self.symbols.insert(symbol, region);
                self.idents.insert(ident, (symbol, region));

                Ok(())
            }
        }
    }

    pub fn add_alias(
        &mut self,
        pool: &mut Pool,
        name: Symbol,
        vars: PoolVec<(PoolStr, Variable)>,
        typ: TypeId,
    ) {
        let mut hidden_variables = MutSet::default();
        hidden_variables.extend(typ.variables(pool));

        for loc_var in vars.iter(pool) {
            hidden_variables.remove(&loc_var.1);
        }

        let hidden_variables_vec = PoolVec::with_capacity(hidden_variables.len() as u32, pool);

        for (node_id, var) in hidden_variables_vec.iter_node_ids().zip(hidden_variables) {
            pool[node_id] = var;
        }

        let alias = Alias {
            targs: vars,
            hidden_variables: hidden_variables_vec,
            actual: typ,
        };

        self.aliases.insert(name, alias);
    }

    pub fn contains_alias(&mut self, name: Symbol) -> bool {
        self.aliases.contains_key(&name)
    }
}

impl ShallowClone for Scope {
    fn shallow_clone(&self) -> Self {
        Self {
            idents: self.idents.clone(),
            symbols: self.symbols.clone(),
            aliases: self
                .aliases
                .iter()
                .map(|(s, a)| (*s, a.shallow_clone()))
                .collect(),
            home: self.home,
        }
    }
}