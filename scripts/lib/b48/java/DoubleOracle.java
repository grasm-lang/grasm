// The oracle for packages/javadouble: Double.toString and Float.toString.
// Each input line is `d <hex bits>` or `f <hex bits>`; each answer is the
// string Java writes.
import java.io.*;

public class DoubleOracle {
    public static void main(String[] a) throws Exception {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in));
        StringBuilder out = new StringBuilder();
        String line;
        while ((line = in.readLine()) != null) {
            if (line.isEmpty()) continue;
            String[] f = line.split(" ");
            long bits = Long.parseUnsignedLong(f[1], 16);
            out.append(f[0].equals("d") ? Double.toString(Double.longBitsToDouble(bits)) : Float.toString(Float.intBitsToFloat((int) bits))).append('\n');
        }
        System.out.print(out);
    }
}
