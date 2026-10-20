// The oracle for packages/javabigdecimal. Each input line is an operation
// with decimal arguments in hex; each answer is the result's printed forms,
// scale, precision, unscaled value, hash and sign, or `nfe` / `arith` for
// Java's NumberFormatException / ArithmeticException.
import java.io.*;
import java.math.*;
import java.nio.charset.StandardCharsets;

public class BigOracle {
    static String unhex(String h) {
        if (h.equals("-")) return "";
        byte[] b = new byte[h.length() / 2];
        for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(h.substring(2 * i, 2 * i + 2), 16);
        return new String(b, StandardCharsets.UTF_8);
    }
    static BigDecimal d(String h) { return new BigDecimal(unhex(h)); }
    static String desc(BigDecimal x) {
        return x.toString() + " " + x.toEngineeringString() + " " + x.toPlainString() + " " + x.scale() + " " + x.precision() + " "
            + x.unscaledValue() + " " + x.hashCode() + " " + x.signum();
    }
    static MathContext mc(String p, String m) { return new MathContext(Integer.parseInt(p), RoundingMode.valueOf(m)); }
    static String run(String[] f) {
        switch (f[0]) {
            case "s": {
                BigDecimal x = d(f[1]);
                return desc(x) + " " + Long.toHexString(Double.doubleToRawLongBits(x.doubleValue())) + " " + x.stripTrailingZeros() + " "
                    + x.toBigInteger() + " " + x.longValue() + " " + x.ulp();
            }
            case "nbd": return desc(new BigDecimal(Double.longBitsToDouble(Long.parseUnsignedLong(f[1], 16))));
            case "vod": return desc(BigDecimal.valueOf(Double.longBitsToDouble(Long.parseUnsignedLong(f[1], 16))));
            case "add": return desc(d(f[1]).add(d(f[2])));
            case "sub": return desc(d(f[1]).subtract(d(f[2])));
            case "mul": return desc(d(f[1]).multiply(d(f[2])));
            case "div": return desc(d(f[1]).divide(d(f[2])));
            case "dti": return desc(d(f[1]).divideToIntegralValue(d(f[2])));
            case "rem": return desc(d(f[1]).remainder(d(f[2])));
            case "cmp": {
                BigDecimal a = d(f[1]), b = d(f[2]);
                return a.compareTo(b) + " " + a.equals(b) + " " + a.min(b) + " " + a.max(b);
            }
            case "divs": return desc(d(f[1]).divide(d(f[2]), Integer.parseInt(f[3]), RoundingMode.valueOf(f[4])));
            case "divm": return desc(d(f[1]).divide(d(f[2]), RoundingMode.valueOf(f[3])));
            case "addc": return desc(d(f[1]).add(d(f[2]), mc(f[3], f[4])));
            case "subc": return desc(d(f[1]).subtract(d(f[2]), mc(f[3], f[4])));
            case "mulc": return desc(d(f[1]).multiply(d(f[2]), mc(f[3], f[4])));
            case "divc": return desc(d(f[1]).divide(d(f[2]), mc(f[3], f[4])));
            case "round": return desc(d(f[1]).round(mc(f[2], f[3])));
            case "sets": return desc(d(f[1]).setScale(Integer.parseInt(f[2]), RoundingMode.valueOf(f[3])));
            case "pow": return desc(d(f[1]).pow(Integer.parseInt(f[2])));
            case "mpl": return desc(d(f[1]).movePointLeft(Integer.parseInt(f[2])));
            case "mpr": return desc(d(f[1]).movePointRight(Integer.parseInt(f[2])));
            default: return desc(d(f[1]).scaleByPowerOfTen(Integer.parseInt(f[2])));
        }
    }
    public static void main(String[] x) throws Exception {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in));
        StringBuilder out = new StringBuilder();
        String line;
        while ((line = in.readLine()) != null) {
            if (line.isEmpty()) continue;
            String r;
            try {
                r = run(line.split(" "));
            } catch (NumberFormatException e) {
                r = "nfe";
            } catch (ArithmeticException e) {
                r = "arith";
            }
            out.append(r).append('\n');
        }
        System.out.print(out);
    }
}
