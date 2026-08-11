from datetime import datetime

h = datetime.now().hour
nombre = input("¿Cómo te llamas? ")

if h < 12:
    print(f"¡Buenos días, {nombre}! 🌅")
elif h < 18:
    print(f"¡Buenas tardes, {nombre}! ☀️")
else:
    print(f"¡Buenas noches, {nombre}! 🌙")
