#![no_std]
//! Compile this complete allocator-free workflow for the device ABI. LLVM folds
//! size_of_val into a constant; check_ram.py reads it without running MCU code.
use timetable::{
    Date, Error, TripIndex, TripSlot, memory,
    query::{DepartureBuffer, DepartureSlot},
    stream, zip,
};

pub async fn workflow<const TRIPS: usize>(metadata: &[u8], data: &[u8]) -> Result<usize, Error> {
    let mut workspace = zip::Workspace::new();
    let mut trip_slots = [TripSlot::<32>::EMPTY; TRIPS];
    let mut trips = TripIndex::new(&mut trip_slots);
    let mut service_slots = [stream::ServiceSlot::<32>::EMPTY; 128];
    let mut services = stream::Services::new(&mut service_slots);
    let mut event_slots = [DepartureSlot::<32>::EMPTY; 256];
    let mut output = DepartureBuffer::new(&mut event_slots);
    let selection = stream::Selection {
        routes: &["4"],
        date: Some(Date::new(2026, 10, 17)?),
        direction: None,
    };
    let manifest = stream::prepare(
        metadata,
        selection,
        &mut trips,
        &mut services,
        &mut workspace,
    )
    .await?;
    stream::collect(
        data,
        &manifest,
        &trips,
        &["129"],
        &mut workspace,
        &mut output,
    )
    .await?;
    output.sort();
    Ok(output.len())
}

#[inline(never)]
pub fn task_state_bytes_1024() -> usize {
    core::mem::size_of_val(&workflow::<1024>(&[], &[]))
}
#[inline(never)]
pub fn task_state_bytes_2048() -> usize {
    core::mem::size_of_val(&workflow::<2048>(&[], &[]))
}
#[inline(never)]
pub fn task_state_bytes_4096() -> usize {
    core::mem::size_of_val(&workflow::<4096>(&[], &[]))
}
#[inline(never)]
pub fn reserves_bytes() -> usize {
    let r = memory::Reserves::EMBEDDED;
    r.tls_rx + r.tls_tx + r.http + r.network + r.call_stack + r.application
}
