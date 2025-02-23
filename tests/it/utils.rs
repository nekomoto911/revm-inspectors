use alloy_primitives::{Address, Bytes, U256};
use colorchoice::ColorChoice;
use revm::{
    db::{CacheDB, EmptyDB},
    inspector_handle_register,
    primitives::{
        BlockEnv, EVMError, Env, EnvWithHandlerCfg, ExecutionResult, HandlerCfg, ResultAndState,
        SpecId, TransactTo, TxEnv,
    },
    Database, DatabaseCommit, GetInspector,
};
use revm_inspectors::tracing::{
    TraceWriter, TraceWriterConfig, TracingInspector, TracingInspectorConfig,
};
use std::convert::Infallible;

type TestDb = CacheDB<EmptyDB>;

#[derive(Clone, Debug)]
pub struct TestEvm {
    pub db: TestDb,
    pub env: EnvWithHandlerCfg,
}

impl Default for TestEvm {
    fn default() -> Self {
        Self::new()
    }
}

impl TestEvm {
    pub fn new() -> Self {
        let db = CacheDB::new(EmptyDB::default());
        let env = EnvWithHandlerCfg::new(
            Box::new(Env {
                block: BlockEnv { gas_limit: U256::MAX, ..Default::default() },
                tx: TxEnv { gas_limit: u64::MAX, gas_price: U256::ZERO, ..Default::default() },
                ..Default::default()
            }),
            HandlerCfg::new(SpecId::CANCUN),
        );
        Self { db, env }
    }

    pub fn new_with_spec_id(spec_id: SpecId) -> Self {
        let mut evm = Self::new();
        evm.env.handler_cfg.spec_id = spec_id;
        evm
    }

    pub fn env_with_tx(&self, tx_env: TxEnv) -> EnvWithHandlerCfg {
        let mut env = self.env.clone();
        env.tx = tx_env;
        env
    }

    pub fn simple_deploy(&mut self, data: Bytes) -> Address {
        self.deploy(data, TracingInspector::new(TracingInspectorConfig::default_geth()))
            .expect("failed to deploy contract")
    }

    pub fn deploy<I: for<'a> GetInspector<&'a mut TestDb>>(
        &mut self,
        data: Bytes,
        inspector: I,
    ) -> Result<Address, EVMError<Infallible>> {
        let (_, address) = self.try_deploy(data, inspector)?;
        Ok(address.expect("failed to deploy contract"))
    }

    pub fn try_deploy<I: for<'a> GetInspector<&'a mut TestDb>>(
        &mut self,
        data: Bytes,
        inspector: I,
    ) -> Result<(ExecutionResult, Option<Address>), EVMError<Infallible>> {
        self.env.tx.data = data;
        self.env.tx.transact_to = TransactTo::Create;

        let (ResultAndState { result, state }, env) = self.inspect(inspector)?;
        self.db.commit(state);
        self.env = env;
        match &result {
            ExecutionResult::Success { output, .. } => {
                let address = output.address().copied();
                Ok((result, address))
            }
            _ => Ok((result, None)),
        }
    }

    pub fn call<I: for<'a> GetInspector<&'a mut TestDb>>(
        &mut self,
        address: Address,
        data: Bytes,
        inspector: I,
    ) -> Result<ExecutionResult, EVMError<Infallible>> {
        self.env.tx.data = data;
        self.env.tx.transact_to = TransactTo::Call(address);
        let (ResultAndState { result, state }, env) = self.inspect(inspector)?;
        self.db.commit(state);
        self.env = env;
        Ok(result)
    }

    pub fn inspect<I: for<'a> GetInspector<&'a mut TestDb>>(
        &mut self,
        inspector: I,
    ) -> Result<(ResultAndState, EnvWithHandlerCfg), EVMError<Infallible>> {
        inspect(&mut self.db, self.env.clone(), inspector)
    }
}

/// Executes the [EnvWithHandlerCfg] against the given [Database] without committing state changes.
pub fn inspect<DB, I>(
    db: DB,
    env: EnvWithHandlerCfg,
    inspector: I,
) -> Result<(ResultAndState, EnvWithHandlerCfg), EVMError<DB::Error>>
where
    DB: Database,
    I: GetInspector<DB>,
{
    let mut evm = revm::Evm::builder()
        .with_db(db)
        .with_external_context(inspector)
        .with_env_with_handler_cfg(env)
        .append_handler_register(inspector_handle_register)
        .build();
    let res = evm.transact()?;
    let (_, env) = evm.into_db_and_env_with_handler_cfg();
    Ok((res, env))
}

pub fn write_traces(tracer: &TracingInspector) -> String {
    write_traces_with(tracer, TraceWriterConfig::new().color_choice(ColorChoice::Never))
}

pub fn write_traces_with(tracer: &TracingInspector, config: TraceWriterConfig) -> String {
    let mut w = TraceWriter::with_config(Vec::<u8>::new(), config);
    w.write_arena(tracer.traces()).expect("failed to write traces to Vec<u8>");
    String::from_utf8(w.into_writer()).expect("trace writer wrote invalid UTF-8")
}

pub fn print_traces(tracer: &TracingInspector) {
    // Use `println!` so that the output is captured by the test runner.
    println!("{}", write_traces_with(tracer, TraceWriterConfig::new()));
}

/// Deploys a contract with the given code and deployer address.
pub fn deploy_contract(code: Bytes, deployer: Address, spec_id: SpecId) -> (Address, TestEvm) {
    let mut evm = TestEvm::new();

    evm.env.tx.caller = deployer;
    evm.env.handler_cfg = HandlerCfg::new(spec_id);

    (evm.simple_deploy(code), evm)
}
