// The oracle for packages/javabiginteger: Java 17's java.math.BigInteger.
// Numbers travel in decimal; text and bytes in hex, '-' when empty.
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;

public class BigIntegerOracle {
    static String hx(byte[] b) {
        if (b.length == 0) return "-";
        StringBuilder s = new StringBuilder();
        for (byte x : b) s.append(String.format("%02x", x & 0xff));
        return s.toString();
    }

    static byte[] unhex(String h) {
        if (h.equals("-")) return new byte[0];
        byte[] b = new byte[h.length() / 2];
        for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(h.substring(2 * i, 2 * i + 2), 16);
        return b;
    }

    static String line(String[] f) {
        switch (f[0]) {
            case "pa": return new BigInteger(new String(unhex(f[1]), StandardCharsets.UTF_8), Integer.parseInt(f[2])).toString();
            case "ts": return new BigInteger(f[1]).toString(Integer.parseInt(f[2]));
            case "ba": return hx(new BigInteger(f[1]).toByteArray());
            case "fb": return new BigInteger(unhex(f[1])).toString();
            case "bl": { BigInteger a = new BigInteger(f[1]); return a.bitLength() + " " + a.bitCount() + " " + a.getLowestSetBit(); }
            case "bw": {
                BigInteger a = new BigInteger(f[2]);
                BigInteger b = new BigInteger(f[3]);
                switch (f[1]) {
                    case "and": return a.and(b).toString();
                    case "or": return a.or(b).toString();
                    case "xor": return a.xor(b).toString();
                    case "andnot": return a.andNot(b).toString();
                    default: return a.not().toString();
                }
            }
            case "sh": { BigInteger a = new BigInteger(f[1]); int k = Integer.parseInt(f[2]); return a.shiftLeft(k) + " " + a.shiftRight(k); }
            case "tb": {
                BigInteger a = new BigInteger(f[1]);
                int k = Integer.parseInt(f[2]);
                return a.testBit(k) + " " + a.setBit(k) + " " + a.clearBit(k) + " " + a.flipBit(k);
            }
            case "ar": {
                BigInteger a = new BigInteger(f[2]);
                switch (f[1]) {
                    case "div": return a.divide(new BigInteger(f[3])).toString();
                    case "rem": return a.remainder(new BigInteger(f[3])).toString();
                    case "mod": return a.mod(new BigInteger(f[3])).toString();
                    case "gcd": return a.gcd(new BigInteger(f[3])).toString();
                    case "inv": return a.modInverse(new BigInteger(f[3])).toString();
                    case "pow": return a.pow(Integer.parseInt(f[3])).toString();
                    default: return a.sqrt().toString();
                }
            }
            case "mp": return new BigInteger(f[1]).modPow(new BigInteger(f[2]), new BigInteger(f[3])).toString();
            default: { BigInteger a = new BigInteger(f[1]); return a.longValue() + " " + a.intValue() + " " + a.longValueExact(); }
        }
    }

    public static void main(String[] args) throws Exception {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        StringBuilder out = new StringBuilder();
        String l;
        while ((l = in.readLine()) != null) {
            if (l.isEmpty()) continue;
            String r;
            try {
                r = line(l.split(" "));
            } catch (NumberFormatException e) {
                r = "Enf:" + hx(e.getMessage().getBytes(StandardCharsets.UTF_8));
            } catch (ArithmeticException e) {
                r = "Ear:" + hx(e.getMessage().getBytes(StandardCharsets.UTF_8));
            }
            out.append(r).append('\n');
        }
        System.out.print(out);
    }
}
