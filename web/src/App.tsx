import '@mantine/core/styles.css';
import { MantineProvider, Title, Text, Container } from '@mantine/core';

export function App() {
  return (
    <MantineProvider defaultColorScheme="auto">
      <Container py="xl">
        <Title>LibreBooks</Title>
        <Text c="dimmed">Open-source double-entry accounting.</Text>
      </Container>
    </MantineProvider>
  );
}
