use gbf_abi::interrupt::InterruptPolicy;
use gbf_asm::builder::Builder;
use gbf_asm::encoder::encode_instr;
use gbf_asm::lowering::lower_pre_layout_ops;
use gbf_asm::section::{SectionPrivilege, SectionRole};
use gbf_asm::symbols::{SymbolName, SymbolTable};
use gbf_runtime::banking::{
    BankingPreLayoutLowering, LeaseLifetime, ReturnRomBank, ReturnState, ValidatedBankLeaseSpec,
    lease_rom_switchable, release_bank,
};

fn main() {
    let mut builder = Builder::new(
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("banking", "demo").expect("section name"),
    )
    .with_section_privilege(SectionPrivilege::privileged());

    let guard = lease_rom_switchable(
        &mut builder,
        ValidatedBankLeaseSpec::for_rom_switchable(3, LeaseLifetime::Slice).expect("valid lease"),
    )
    .expect("lease");
    release_bank(&mut builder, guard, ReturnState::Rom(ReturnRomBank::Bank1)).expect("release");

    let lowerer = BankingPreLayoutLowering::new(
        InterruptPolicy::ShortCriticalSection,
        LeaseLifetime::Slice,
        Default::default(),
    );
    let lowered = lower_pre_layout_ops(vec![builder.finish()], &lowerer, &SymbolTable::new())
        .expect("banking lowering");
    let bytes: Vec<u8> = lowered[0]
        .instrs
        .iter()
        .flat_map(|item| encode_instr(&item.data).expect("instruction encodes"))
        .collect();

    println!(
        "{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
}
