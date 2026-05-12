#![cfg(feature = "burn-adapter")]

use std::sync::atomic::{AtomicU64, Ordering};

use burn::backend::{Autodiff, NdArray};
use burn::tensor::Tensor;
use burn::tensor::backend::Backend;
use gbf_train::teacher::{
    DenseTeacherModel, TeacherStorageFingerprint, TeacherStorageIdentity, TeacherWeightFingerprint,
    freeze_teacher,
};

type TestBackend = Autodiff<NdArray>;

static NEXT_STORAGE_ID: AtomicU64 = AtomicU64::new(10_000);

#[test]
fn freeze_teacher_detaches_burn_parameters_from_autodiff_graph() {
    let device = <TestBackend as Backend>::Device::default();
    let student = BurnLinearTeacher::new([2.0, -3.0], &device);

    assert!(student.teacher_requires_grad());

    let teacher = freeze_teacher(&student).unwrap();

    assert!(!teacher.requires_grad());
    assert_eq!(
        teacher.weight_fingerprint(),
        &student.teacher_weight_fingerprint()
    );
    assert_eq!(
        teacher.storage_fingerprint(),
        &student.teacher_storage_fingerprint()
    );

    let input = Tensor::<TestBackend, 1>::from_floats([4.0, 5.0], &device).require_grad();
    let output = teacher.forward_no_grad(input.clone()).unwrap();
    let grads = output.loss.backward();

    assert!(input.grad(&grads).is_some());
    assert!(!output.teacher_weight.is_require_grad());
    assert!(output.teacher_weight.grad(&grads).is_none());
}

#[derive(Clone)]
struct BurnLinearTeacher {
    weight: Tensor<TestBackend, 1>,
    storage_identity: u64,
}

impl BurnLinearTeacher {
    fn new(weights: [f32; 2], device: &<TestBackend as Backend>::Device) -> Self {
        Self {
            weight: Tensor::<TestBackend, 1>::from_floats(weights, device).require_grad(),
            storage_identity: NEXT_STORAGE_ID.fetch_add(1, Ordering::Relaxed),
        }
    }
}

impl DenseTeacherModel for BurnLinearTeacher {
    type Input = Tensor<TestBackend, 1>;
    type Output = BurnTeacherOutput;
    type ForwardError = BurnTeacherError;

    fn detach_for_teacher(&mut self) {
        self.weight = self.weight.clone().detach().set_require_grad(false);
        self.storage_identity = NEXT_STORAGE_ID.fetch_add(1, Ordering::Relaxed);
    }

    fn forward_no_grad(&self, input: Self::Input) -> Result<Self::Output, Self::ForwardError> {
        let teacher_weight = self.weight.clone().detach().set_require_grad(false);
        let loss = (teacher_weight.clone() * input).sum();

        Ok(BurnTeacherOutput {
            loss,
            teacher_weight,
        })
    }

    fn teacher_weight_fingerprint(&self) -> TeacherWeightFingerprint {
        TeacherWeightFingerprint::new(tensor_payload_bytes(&self.weight)).unwrap()
    }

    fn teacher_storage_fingerprint(&self) -> TeacherStorageFingerprint {
        let mut bytes = Vec::from("burn-linear-teacher:f32:rank1:");
        bytes.extend_from_slice(&2usize.to_le_bytes());
        bytes.extend_from_slice(&tensor_payload_bytes(&self.weight));
        TeacherStorageFingerprint::new(bytes).unwrap()
    }

    fn teacher_storage_identity(&self) -> TeacherStorageIdentity {
        TeacherStorageIdentity::new(self.storage_identity.to_le_bytes()).unwrap()
    }

    fn teacher_requires_grad(&self) -> bool {
        self.weight.is_require_grad()
    }
}

struct BurnTeacherOutput {
    loss: Tensor<TestBackend, 1>,
    teacher_weight: Tensor<TestBackend, 1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BurnTeacherError;

fn tensor_payload_bytes(tensor: &Tensor<TestBackend, 1>) -> Vec<u8> {
    let values = tensor.to_data().to_vec::<f32>().unwrap();
    let mut bytes = Vec::with_capacity(values.len() * size_of::<f32>());
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}
