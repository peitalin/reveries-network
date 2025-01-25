use umbral_pre::{
    decrypt_original, decrypt_reencrypted, encrypt,
    SecretBox,
    SecretKeyFactory,
    reencrypt, Capsule, KeyFrag, PublicKey, SecretKey, Signer
};


// Proxy Re-Encryption Key
#[derive(Clone)]
pub struct UmbralKey {
    pub secret_key: SecretKey, // make private and accessible only in NodeClient
    pub public_key: PublicKey,
    pub signer: Signer,
    pub verifying_pk: PublicKey,
}

impl UmbralKey {
    pub fn new(seed: Option<&[u8]>) -> Self {

        // Key Generation (on Alice's side)
        let sk = match seed {
            // deterministic with seed
            Some(seed) => {
                let secret_key_factory = SecretKeyFactory::random();
                let sk = secret_key_factory.make_key(seed);
                sk
            }
            // or generate random secret key
            None => {
                SecretKey::random()
            }
        };

        let pk = sk.public_key();
        let signer = Signer::new(SecretKey::random());
        let verifying_pk = signer.verifying_key();

        UmbralKey {
            secret_key: sk,
            public_key: pk,
            signer: signer,
            verifying_pk: verifying_pk
        }
    }

    pub fn check_ciphertext_decryptable(
        &self,
        capsule: &Capsule,
        ciphertext: &Box<[u8]>
    ) -> Box<[u8]> {

        // alice can decrypt her own capsule and ciphertext
        let plaintext_alice = decrypt_original(
            &self.secret_key,
            &capsule,
            &ciphertext
        ).unwrap();

        println!("plaintext_alice: {:?}", String::from_utf8(plaintext_alice.to_vec()));
        plaintext_alice
    }
}


pub fn run_reencrypt_example() -> anyhow::Result<String> {

    let plaintext = "to be or not to be".as_bytes();

    println!("\nProxy reencryption example");
    println!("Handling vessel/MPC mode logic");

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
        plaintext
    ).unwrap();

    assert_eq!(
        &alice_pre_key.check_ciphertext_decryptable(&capsule, &ciphertext) as &[u8],
        plaintext
    );

    // Alice generates reencryption key fragments for Bob (MPC node)
    let shares = 3;
    let threshold = 2;
    let kfrags = generate_pre_kfrags(
        &alice_pre_key.secret_key,
        &bob_pk,
        &alice_pre_key.signer,
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


    // Simulate network transfer from Usrula
    let cfrag0 = verified_cfrag0.clone().unverify();
    let cfrag1 = verified_cfrag1.clone().unverify();

    // Finally, Bob opens the capsule by using at least `threshold` cfrags,
    // and then decrypts the re-encrypted ciphertext.
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

    let plaintext_bob = decrypt_reencrypted(
       &bob_sk,
       &alice_pk,
       &capsule,
       [verified_cfrag0, verified_cfrag1],
       ciphertext
    ).unwrap();

    assert_eq!(&plaintext_bob as &[u8], plaintext);

    let plaintext_bob_str= String::from_utf8(plaintext_bob.to_vec())?;
    println!("plaintext_bob: {:?}", plaintext_bob_str);

    Ok(plaintext_bob_str)
}


pub fn generate_pre_kfrags(
    alice_sk: &SecretKey,
    bob_pk: &PublicKey,
    signer: &Signer,
    threshold: usize,
    shares: usize,
) -> Vec<KeyFrag> {

    let verified_kfrags = umbral_pre::generate_kfrags(
        alice_sk,
        bob_pk,
        signer,
        threshold,
        shares,
        true, // sign_delegating_key
        true, // sign_receiving_key
    );

    verified_kfrags.iter()
        .map(|verified_keyfrag| verified_keyfrag.clone().unverify())
        .collect::<Vec<KeyFrag>>()
}