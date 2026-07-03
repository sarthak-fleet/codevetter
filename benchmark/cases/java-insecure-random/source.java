// Case: Using java.util.Random for security-sensitive tokens.
import java.util.Random;

public class TokenGenerator {
    private static final Random RNG = new Random();

    // BUG: java.util.Random is a predictable PRNG. Session/reset tokens derived
    // from it can be guessed by an attacker who observes one output, because
    // the internal seed is recoverable. Use SecureRandom instead.
    public static String resetToken() {
        return Long.toHexString(RNG.nextLong());
    }
}
