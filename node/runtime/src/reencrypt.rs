use color_eyre::{eyre, Result};
use umbral_pre::{
    decrypt_original,
    decrypt_reencrypted,
    encrypt,
    reencrypt,
    PublicKey,
    SecretKey,
    ReencryptionError,
    SecretKeyFactory,
    Signer,
};
pub use umbral_pre::{
    Capsule,
    KeyFrag,
    VerifiedCapsuleFrag
};

// Proxy Re-Encryption Key
#[derive(Clone)]
pub struct UmbralKey {
    secret_key: SecretKey, // keep private
    pub public_key: PublicKey,
    pub signer: Signer,
    pub verifying_pk: PublicKey,
}

impl UmbralKey {
    pub fn new(seed: Option<&[u8]>) -> Self {
        // Key Generation (on Alice's side)
        let secret_key = match seed {
            // randomly generated secret key
            None => SecretKey::random(),
            // deterministic with seed
            Some(seed) => {
                match SecretKeyFactory::from_secure_randomness(seed) {
                    Ok(factory) => {
                        let secret_key = factory.make_key(seed);
                        secret_key
                    }
                    Err(e) => panic!("Invalid Umbral seed: {}", e)
                }
            }
        };

        let public_key = secret_key.public_key();
        let signer = Signer::new(SecretKey::random());
        let verifying_pk = signer.verifying_key();

        UmbralKey {
            secret_key: secret_key,
            public_key: public_key,
            signer: signer,
            verifying_pk: verifying_pk
        }
    }

    pub fn encrypt_bytes(&self, plaintext: &Vec<u8>) -> Result<(Capsule, Box<[u8]>)> {
        let (capsule, ciphertext) = umbral_pre::encrypt(
            &self.public_key,
            &plaintext
        ).map_err(|e| eyre::anyhow!(e.to_string()))?;

        Ok((capsule, ciphertext))
    }

    pub fn decrypt_original(&self, capsule: &Capsule, ciphertext: &Box<[u8]>) -> Result<Box<[u8]>> {
        let result = umbral_pre::decrypt_original(
            &self.secret_key,
            &capsule,
            &ciphertext
        ).map_err(|e| eyre::anyhow!(e.to_string()))?;

        Ok(result)
    }

    pub fn decrypt_reencrypted(
        &self,
        delegator_pubkey: &PublicKey, // alice public key
        capsule: &Capsule,
        verified_cfrags: impl IntoIterator<Item = VerifiedCapsuleFrag>,
        ciphertext: Box<[u8]>
    ) -> Result<Box<[u8]>, ReencryptionError> {
        umbral_pre::decrypt_reencrypted(
            &self.secret_key, // bob
            delegator_pubkey, // alice
            capsule,
            verified_cfrags,
            ciphertext
        )
    }

    pub fn check_ciphertext_decryptable(
        &self,
        capsule: &Capsule,
        ciphertext: &Box<[u8]>,
        plaintext_original: &[u8],
    ) {
        // alice can decrypt her own capsule and ciphertext
        let decrypted_plaintext = self.decrypt_original(&capsule, &ciphertext).unwrap();
        assert_eq!(
            &decrypted_plaintext as &[u8],
            plaintext_original
        );
    }

    pub fn generate_pre_kfrags(
        &self,
        bob_pk: &PublicKey,
        threshold: usize,
        shares: usize,
    ) -> Vec<KeyFrag> {

        let verified_kfrags = umbral_pre::generate_kfrags(
            &self.secret_key, // alice secret key
            bob_pk,
            &self.signer,
            // `signer` is used to sign the resulting [`KeyFrag`](`crate::KeyFrag`) objects,
            // which can be later verified by the associated public key.
            threshold,
            shares,
            true, // sign_delegating_key
            true, // sign_receiving_key
            // If `sign_delegating_key` or `sign_receiving_key` are `true`,
            // the reencrypting party will be able to verify that a [`KeyFrag`](`crate::KeyFrag`)
            // corresponds to given delegating or receiving public keys
            // by supplying them to [`KeyFrag::verify()`](`crate::KeyFrag::verify`).
        );

        verified_kfrags.iter()
            .map(|verified_keyfrag| verified_keyfrag.clone().unverify())
            .collect::<Vec<KeyFrag>>()
    }
}

pub fn run_reencrypt_example() -> Result<(Box<[u8]>, Box<[u8]>, Vec<u8>)> {
    // https://docs.rs/umbral-pre/latest/umbral_pre/
    println!("Proxy reencryption test");
    // Original plaintext
    let plaintext_original = "to be or not to be".as_bytes();
    // Key Generation (on Alice's side)
    let alice_pre_key = UmbralKey::new(None);
    let alice_pk = alice_pre_key.public_key;
    let verifying_pk = alice_pre_key.verifying_pk;
    println!("alice_pk : {}", alice_pk);
    println!("signer verifying_pk: {}", verifying_pk);

    // Key Generation (on Bob's side)
    let bob_pre_key = UmbralKey::new(None);
    let bob_sk = bob_pre_key.secret_key;
    let bob_pk = bob_pre_key.public_key;
    println!("bob_pk: {}\n", bob_pk);

    let (capsule, ciphertext) = encrypt(
        &alice_pre_key.public_key,
        plaintext_original
    ).unwrap();

    alice_pre_key.check_ciphertext_decryptable(&capsule, &ciphertext, plaintext_original);

    // Alice generates reencryption key fragments for Bob (MPC node)
    let shares = 3;
    let threshold = 2;
    let kfrags = alice_pre_key.generate_pre_kfrags(
        &bob_pk,
        threshold,
        shares
    );

    // Simulate network transfer to Usulas
    let kfrag0 = kfrags[0].clone();
    let kfrag1 = kfrags[1].clone();

    // Ursula 0
    let verified_kfrag0 = kfrag0.verify(&verifying_pk, Some(&alice_pk), Some(&bob_pk)).unwrap();
    let verified_cfrag0 = reencrypt(&capsule, verified_kfrag0);

    // Ursula 1
    let verified_kfrag1 = kfrag1.verify(&verifying_pk, Some(&alice_pk), Some(&bob_pk)).unwrap();
    let verified_cfrag1 = reencrypt(&capsule, verified_kfrag1);

    // Simulate network transfer from Usrula to Bob
    let cfrag0 = verified_cfrag0.clone().unverify();
    let cfrag1 = verified_cfrag1.clone().unverify();

    // Bob must check that cfrags are valid
    let verified_cfrag0 = cfrag0.verify(
        &capsule,
        &verifying_pk,
        &alice_pk,
        &bob_pk
    ).unwrap();

    let verified_cfrag1 = cfrag1.verify(
        &capsule,
        &verifying_pk,
        &alice_pk,
        &bob_pk
    ).unwrap();

    // Finally, Bob opens the capsule by using at least `threshold` cfrags,
    // and then decrypts the re-encrypted ciphertext.
    let plaintext_bob = decrypt_reencrypted(
       &bob_sk,
       &alice_pk,
       &capsule,
       [verified_cfrag0, verified_cfrag1],
       ciphertext.clone()
    ).unwrap();

    let plaintext_alice = decrypt_original(
        &alice_pre_key.secret_key,
        &capsule,
        ciphertext
    ).unwrap();

    Ok((plaintext_bob, plaintext_alice, plaintext_original.to_vec()))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pre_encryption() -> Result<()> {

        let (
            plaintext_bob,
            plaintext_alice,
            plaintext_original,
        ) = run_reencrypt_example()?;

        assert_eq!(&plaintext_bob as &[u8], plaintext_original);
        assert_eq!(&plaintext_alice as &[u8], plaintext_original);

        let plaintext_bob_str= String::from_utf8(plaintext_bob.to_vec())?;
        let plaintext_alice_str= String::from_utf8(plaintext_alice.to_vec())?;

        println!("plaintext_bob: {:?}", plaintext_bob_str);
        println!("plaintext_alice: {:?}", plaintext_alice_str);

        Ok(())
    }
}
