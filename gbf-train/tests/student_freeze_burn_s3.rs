#![cfg(feature = "burn-adapter")]

mod student {
    pub mod freeze {
        use std::cell::RefCell;
        use std::rc::Rc;
        use std::sync::atomic::{AtomicU64, Ordering};

        use burn::backend::{Autodiff, NdArray};
        use burn::tensor::Tensor;
        use burn::tensor::backend::Backend;
        use gbf_train::student::{
            HardTernaryStudentModel, StudentFreezeError, StudentStorageFingerprint,
            StudentWeightFingerprint, freeze_student_as_artifact,
        };

        type TestBackend = Autodiff<NdArray>;

        static NEXT_STORAGE_ID: AtomicU64 = AtomicU64::new(20_000);

        #[test]
        fn freeze_student_detaches_burn_parameters_from_autodiff_graph() {
            let device = <TestBackend as Backend>::Device::default();
            let student = BurnLinearStudent::new([2.0, -3.0], &device);

            assert!(student.student_requires_grad());

            let frozen = freeze_student_as_artifact(&student).unwrap();

            assert!(!frozen.requires_grad());
            assert!(!frozen.snapshot().student_requires_grad());
            assert_eq!(
                frozen.weight_fingerprint(),
                &student.student_weight_fingerprint()
            );
            assert_eq!(
                frozen.storage_fingerprint(),
                &student.student_storage_fingerprint()
            );
            assert_ne!(
                frozen.snapshot().student_storage_identity(),
                student.student_storage_identity()
            );

            let input = Tensor::<TestBackend, 1>::from_floats([4.0, 5.0], &device).require_grad();
            let output = frozen.snapshot().forward_no_grad(input.clone());
            let grads = output.loss.backward();

            assert!(input.grad(&grads).is_some());
            assert!(!output.student_weight.is_require_grad());
            assert!(output.student_weight.grad(&grads).is_none());
        }

        #[test]
        fn freeze_student_rejects_noop_detach_that_keeps_shared_storage() {
            let student = NoOpSharedStudent {
                weights: Rc::new(RefCell::new(vec![1.0, -1.0])),
                requires_grad: false,
            };

            assert_eq!(
                freeze_student_as_artifact(&student).unwrap_err(),
                StudentFreezeError::SharedStorage
            );
        }

        #[test]
        fn freeze_student_rejects_detach_that_mutates_weights() {
            let student = MutatingDetachStudent(ToyStudent::new([1.0, 2.0], true));

            assert!(matches!(
                freeze_student_as_artifact(&student).unwrap_err(),
                StudentFreezeError::FingerprintMismatch { .. }
            ));
        }

        #[test]
        fn freeze_student_rejects_detach_that_mutates_storage_fingerprint() {
            let student = StorageMutatingDetachStudent {
                inner: ToyStudent::new([1.0, 2.0], true),
                storage_generation: 0,
            };

            assert!(matches!(
                freeze_student_as_artifact(&student).unwrap_err(),
                StudentFreezeError::StorageFingerprintMismatch { .. }
            ));
        }

        #[test]
        fn freeze_student_rejects_snapshot_that_still_requires_grad() {
            let student = NoDetachStudent(ToyStudent::new([1.0, 2.0], true));

            assert_eq!(
                freeze_student_as_artifact(&student).unwrap_err(),
                StudentFreezeError::FrozenSnapshotRequiresGrad
            );
        }

        #[derive(Clone)]
        struct BurnLinearStudent {
            weight: Tensor<TestBackend, 1>,
            storage_identity: u64,
        }

        impl BurnLinearStudent {
            fn new(weights: [f32; 2], device: &<TestBackend as Backend>::Device) -> Self {
                Self {
                    weight: Tensor::<TestBackend, 1>::from_floats(weights, device).require_grad(),
                    storage_identity: NEXT_STORAGE_ID.fetch_add(1, Ordering::Relaxed),
                }
            }

            fn forward_no_grad(&self, input: Tensor<TestBackend, 1>) -> BurnStudentOutput {
                let student_weight = self.weight.clone().detach().set_require_grad(false);
                let loss = (student_weight.clone() * input).sum();
                BurnStudentOutput {
                    loss,
                    student_weight,
                }
            }
        }

        impl HardTernaryStudentModel for BurnLinearStudent {
            fn detach_for_student(&mut self) {
                self.weight = self.weight.clone().detach().set_require_grad(false);
                self.storage_identity = NEXT_STORAGE_ID.fetch_add(1, Ordering::Relaxed);
            }

            fn student_weight_fingerprint(&self) -> StudentWeightFingerprint {
                StudentWeightFingerprint::new(tensor_payload_bytes(&self.weight)).unwrap()
            }

            fn student_storage_fingerprint(&self) -> StudentStorageFingerprint {
                let mut bytes = Vec::from("burn-linear-student:f32:rank1:");
                bytes.extend_from_slice(&2usize.to_le_bytes());
                bytes.extend_from_slice(&tensor_payload_bytes(&self.weight));
                StudentStorageFingerprint::new(bytes).unwrap()
            }

            fn student_storage_identity(&self) -> usize {
                self.storage_identity as usize
            }

            fn student_requires_grad(&self) -> bool {
                self.weight.is_require_grad()
            }
        }

        struct BurnStudentOutput {
            loss: Tensor<TestBackend, 1>,
            student_weight: Tensor<TestBackend, 1>,
        }

        #[derive(Clone, Debug)]
        struct ToyStudent {
            weights: Vec<f32>,
            requires_grad: bool,
        }

        impl ToyStudent {
            fn new<const N: usize>(weights: [f32; N], requires_grad: bool) -> Self {
                Self {
                    weights: weights.to_vec(),
                    requires_grad,
                }
            }
        }

        impl HardTernaryStudentModel for ToyStudent {
            fn detach_for_student(&mut self) {
                self.requires_grad = false;
            }

            fn student_weight_fingerprint(&self) -> StudentWeightFingerprint {
                weight_fingerprint(&self.weights)
            }

            fn student_storage_fingerprint(&self) -> StudentStorageFingerprint {
                storage_fingerprint(&self.weights)
            }

            fn student_storage_identity(&self) -> usize {
                self.weights.as_ptr() as usize
            }

            fn student_requires_grad(&self) -> bool {
                self.requires_grad
            }
        }

        #[derive(Clone, Debug)]
        struct MutatingDetachStudent(ToyStudent);

        impl HardTernaryStudentModel for MutatingDetachStudent {
            fn detach_for_student(&mut self) {
                self.0.detach_for_student();
                self.0.weights[0] += 1.0;
            }

            fn student_weight_fingerprint(&self) -> StudentWeightFingerprint {
                self.0.student_weight_fingerprint()
            }

            fn student_storage_fingerprint(&self) -> StudentStorageFingerprint {
                self.0.student_storage_fingerprint()
            }

            fn student_storage_identity(&self) -> usize {
                self.0.student_storage_identity()
            }

            fn student_requires_grad(&self) -> bool {
                self.0.student_requires_grad()
            }
        }

        #[derive(Clone, Debug)]
        struct StorageMutatingDetachStudent {
            inner: ToyStudent,
            storage_generation: u8,
        }

        impl HardTernaryStudentModel for StorageMutatingDetachStudent {
            fn detach_for_student(&mut self) {
                self.inner.detach_for_student();
                self.storage_generation += 1;
            }

            fn student_weight_fingerprint(&self) -> StudentWeightFingerprint {
                self.inner.student_weight_fingerprint()
            }

            fn student_storage_fingerprint(&self) -> StudentStorageFingerprint {
                let mut bytes = self.inner.student_storage_fingerprint().bytes().to_vec();
                bytes.extend_from_slice(b":storage-generation:");
                bytes.push(self.storage_generation);
                StudentStorageFingerprint::new(bytes).unwrap()
            }

            fn student_storage_identity(&self) -> usize {
                self.inner.student_storage_identity()
            }

            fn student_requires_grad(&self) -> bool {
                self.inner.student_requires_grad()
            }
        }

        #[derive(Clone, Debug)]
        struct NoDetachStudent(ToyStudent);

        impl HardTernaryStudentModel for NoDetachStudent {
            fn detach_for_student(&mut self) {}

            fn student_weight_fingerprint(&self) -> StudentWeightFingerprint {
                self.0.student_weight_fingerprint()
            }

            fn student_storage_fingerprint(&self) -> StudentStorageFingerprint {
                self.0.student_storage_fingerprint()
            }

            fn student_storage_identity(&self) -> usize {
                self.0.student_storage_identity()
            }

            fn student_requires_grad(&self) -> bool {
                self.0.student_requires_grad()
            }
        }

        #[derive(Clone, Debug)]
        struct NoOpSharedStudent {
            weights: Rc<RefCell<Vec<f32>>>,
            requires_grad: bool,
        }

        impl HardTernaryStudentModel for NoOpSharedStudent {
            fn detach_for_student(&mut self) {}

            fn student_weight_fingerprint(&self) -> StudentWeightFingerprint {
                weight_fingerprint(&self.weights.borrow())
            }

            fn student_storage_fingerprint(&self) -> StudentStorageFingerprint {
                storage_fingerprint(&self.weights.borrow())
            }

            fn student_storage_identity(&self) -> usize {
                Rc::as_ptr(&self.weights) as usize
            }

            fn student_requires_grad(&self) -> bool {
                self.requires_grad
            }
        }

        fn tensor_payload_bytes(tensor: &Tensor<TestBackend, 1>) -> Vec<u8> {
            let values = tensor.to_data().to_vec::<f32>().unwrap();
            let mut bytes = Vec::with_capacity(values.len() * size_of::<f32>());
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes
        }

        fn weight_fingerprint(weights: &[f32]) -> StudentWeightFingerprint {
            StudentWeightFingerprint::new(
                weights
                    .iter()
                    .flat_map(|weight| weight.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        }

        fn storage_fingerprint(weights: &[f32]) -> StudentStorageFingerprint {
            let mut bytes = Vec::from("toy-student:f32:rank1:");
            bytes.extend_from_slice(&weights.len().to_le_bytes());
            for weight in weights {
                bytes.extend_from_slice(&weight.to_le_bytes());
            }
            StudentStorageFingerprint::new(bytes).unwrap()
        }
    }
}
