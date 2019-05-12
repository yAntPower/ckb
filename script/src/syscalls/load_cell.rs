use crate::syscalls::{
    utils::store_data, CellField, Source, INDEX_OUT_OF_BOUND, ITEM_MISSING,
    LOAD_CELL_BY_FIELD_SYSCALL_NUMBER, LOAD_CELL_SYSCALL_NUMBER, SUCCESS,
};
use byteorder::{LittleEndian, WriteBytesExt};
use ckb_core::cell::{CellMeta, ResolvedOutPoint};
use ckb_core::transaction::CellOutput;
use ckb_protocol::{CellOutput as FbsCellOutput, Script as FbsScript};
use ckb_store::LazyLoadCellOutput;
use ckb_vm::{
    registers::{A0, A3, A4, A5, A7},
    Error as VMError, Register, SupportMachine, Syscalls,
};
use flatbuffers::FlatBufferBuilder;
use std::sync::Arc;

pub struct LoadCell<'a, CS> {
    store: Arc<CS>,
    outputs: &'a [CellMeta],
    resolved_inputs: &'a [&'a ResolvedOutPoint],
    resolved_deps: &'a [&'a ResolvedOutPoint],
}

impl<'a, CS: LazyLoadCellOutput + 'a> LoadCell<'a, CS> {
    pub fn new(
        store: Arc<CS>,
        outputs: &'a [CellMeta],
        resolved_inputs: &'a [&'a ResolvedOutPoint],
        resolved_deps: &'a [&'a ResolvedOutPoint],
    ) -> LoadCell<'a, CS> {
        LoadCell {
            store,
            outputs,
            resolved_inputs,
            resolved_deps,
        }
    }

    fn fetch_cell(&self, source: Source, index: usize) -> Result<&'a CellMeta, u8> {
        match source {
            Source::Input => self
                .resolved_inputs
                .get(index)
                .ok_or(INDEX_OUT_OF_BOUND)
                .and_then(|r| r.cell().ok_or(ITEM_MISSING)),
            Source::Output => self.outputs.get(index).ok_or(INDEX_OUT_OF_BOUND),
            Source::Dep => self
                .resolved_deps
                .get(index)
                .ok_or(INDEX_OUT_OF_BOUND)
                .and_then(|r| r.cell().ok_or(ITEM_MISSING)),
        }
    }

    fn load_full<Mac: SupportMachine>(
        &self,
        machine: &mut Mac,
        output: &CellOutput,
    ) -> Result<(u8, usize), VMError> {
        // NOTE: this is a very expensive operation here since we need to copy
        // everything in a cell to a flatbuffer object, serialize the object
        // into a buffer, and then copy requested data to VM memory space. So
        // we should charge cycles proportional to the full Cell size no matter
        // how much data the actual script is requesting, the per-byte cycle charged
        // here, should also be significantly higher than LOAD_CELL_BY_FIELD.
        // Also, while this is debatable, I suggest we charge full cycles for
        // subsequent calls even if we have cache implemented here.
        // TODO: find a way to cache this without consuming too much memory
        let mut builder = FlatBufferBuilder::new();
        let offset = FbsCellOutput::build(&mut builder, &output);
        builder.finish(offset, None);
        let data = builder.finished_data();

        store_data(machine, &data)?;
        Ok((SUCCESS, data.len()))
    }

    fn load_by_field<Mac: SupportMachine>(
        &self,
        machine: &mut Mac,
        output: &CellOutput,
    ) -> Result<(u8, usize), VMError> {
        let field = CellField::parse_from_u64(machine.registers()[A5].to_u64())?;

        let result = match field {
            CellField::Capacity => {
                let mut buffer = vec![];
                buffer.write_u64::<LittleEndian>(output.capacity.as_u64())?;
                store_data(machine, &buffer)?;
                (SUCCESS, buffer.len())
            }
            CellField::Data => {
                store_data(machine, &output.data)?;
                (SUCCESS, output.data.len())
            }
            CellField::DataHash => {
                let hash = output.data_hash();
                let bytes = hash.as_bytes();
                store_data(machine, &bytes)?;
                (SUCCESS, bytes.len())
            }
            CellField::Lock => {
                let mut builder = FlatBufferBuilder::new();
                let offset = FbsScript::build(&mut builder, &output.lock);
                builder.finish(offset, None);
                let data = builder.finished_data();
                store_data(machine, data)?;
                (SUCCESS, data.len())
            }
            CellField::LockHash => {
                let hash = output.lock.hash();
                let bytes = hash.as_bytes();
                store_data(machine, &bytes)?;
                (SUCCESS, bytes.len())
            }
            CellField::Type => match output.type_ {
                Some(ref type_) => {
                    let mut builder = FlatBufferBuilder::new();
                    let offset = FbsScript::build(&mut builder, &type_);
                    builder.finish(offset, None);
                    let data = builder.finished_data();
                    store_data(machine, data)?;
                    (SUCCESS, data.len())
                }
                None => (ITEM_MISSING, 0),
            },
            CellField::TypeHash => match output.type_ {
                Some(ref type_) => {
                    let hash = type_.hash();
                    let bytes = hash.as_bytes();
                    store_data(machine, &bytes)?;
                    (SUCCESS, bytes.len())
                }
                None => (ITEM_MISSING, 0),
            },
        };
        Ok(result)
    }
}

impl<'a, Mac: SupportMachine, CS: LazyLoadCellOutput> Syscalls<Mac> for LoadCell<'a, CS> {
    fn initialize(&mut self, _machine: &mut Mac) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut Mac) -> Result<bool, VMError> {
        let load_by_field = match machine.registers()[A7].to_u64() {
            LOAD_CELL_SYSCALL_NUMBER => false,
            LOAD_CELL_BY_FIELD_SYSCALL_NUMBER => true,
            _ => return Ok(false),
        };
        machine.add_cycles(100)?;

        let index = machine.registers()[A3].to_usize();
        let source = Source::parse_from_u64(machine.registers()[A4].to_u64())?;

        let cell = self.fetch_cell(source, index);
        if cell.is_err() {
            machine.set_register(A0, Mac::REG::from_u8(cell.unwrap_err()));
            return Ok(true);
        }
        let cell = cell.unwrap();
        let output = self.store.lazy_load_cell_output(&cell);

        let (return_code, len) = if load_by_field {
            self.load_by_field(machine, &output)?
        } else {
            self.load_full(machine, &output)?
        };

        machine.add_cycles(len as u64 * 100)?;
        machine.set_register(A0, Mac::REG::from_u8(return_code));
        Ok(true)
    }
}
